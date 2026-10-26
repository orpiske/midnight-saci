//! Plan bundles: the entry document plus every procedure it invokes.
//!
//! [`load`] packages documents from a directory (client side). [`expand`]
//! validates a submitted bundle without touching the filesystem and flattens
//! it into the ordered execution units of a run (backend and worker side).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::ids::UnitKey;
use crate::parser::{Diagnostic, DocumentRole, PlanError, parse_document};
use crate::plan::{Plan, PlanBundle, ProcedureRef, SourceLocation, Unit};
use crate::run::UnitIdentity;

/// One unit in execution order, with its run-unique identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionUnit {
    /// Key, location and classification.
    pub identity: UnitIdentity,
    /// The authored unit.
    pub unit: Unit,
    /// Bundle-relative path of the document defining the unit.
    pub document: String,
    /// Keys of the units whose procedure invocations led here, outermost first.
    pub invoked_by: Vec<UnitKey>,
}

/// A validated bundle and its execution order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedPlan {
    /// The parsed entry document.
    pub plan: Plan,
    /// The parsed procedures, by bundle-relative path.
    pub procedures: BTreeMap<String, Plan>,
    /// Units in execution order.
    pub units: Vec<ExecutionUnit>,
}

/// Reads `entry` and every procedure it invokes, recursively, into a bundle.
///
/// The bundle root is the entry document's directory. Procedure paths are
/// resolved relative to the invoking document.
///
/// # Errors
///
/// Returns diagnostics for unreadable or invalid documents, missing
/// procedures, paths or symlinks that leave the bundle root, and recursive
/// invocations.
pub fn load(entry: &Path) -> Result<PlanBundle, PlanError> {
    let root = entry
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let root = root
        .canonicalize()
        .map_err(|error| io_error(entry, &error))?;
    let name = entry
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut bundle = PlanBundle {
        entry: name.clone(),
        documents: BTreeMap::new(),
    };
    let mut loader = Loader {
        root,
        bundle: &mut bundle,
    };
    loader.read(&name, None)?;
    expand(&bundle)?;
    Ok(bundle)
}

struct Loader<'a> {
    root: PathBuf,
    bundle: &'a mut PlanBundle,
}

impl Loader<'_> {
    /// Reads `path` and, recursively, the documents it invokes.
    fn read(&mut self, path: &str, invoked_at: Option<&SourceLocation>) -> Result<(), PlanError> {
        if self.bundle.documents.contains_key(path) {
            return Ok(());
        }
        let source = self.read_file(path, invoked_at)?;
        let role = if invoked_at.is_some() {
            DocumentRole::Procedure
        } else {
            DocumentRole::Entry
        };
        let plan = parse_document(path, &source, role)?;
        self.bundle.documents.insert(path.to_owned(), source);
        for procedure in plan.units().flat_map(|unit| &unit.procedures) {
            let target = resolve(path, procedure)?;
            self.read(&target, Some(&procedure.location))?;
        }
        Ok(())
    }

    fn read_file(
        &self,
        path: &str,
        invoked_at: Option<&SourceLocation>,
    ) -> Result<String, PlanError> {
        let location = invoked_at
            .cloned()
            .unwrap_or_else(|| SourceLocation::new(path, 1));
        let file = self.root.join(path);
        let canonical = file
            .canonicalize()
            .map_err(|error| single(location.clone(), format!("cannot read `{path}`: {error}")))?;
        if !canonical.starts_with(&self.root) {
            let message = format!("`{path}` resolves outside the bundle directory (symlink?)");
            return Err(single(location, message));
        }
        std::fs::read_to_string(&canonical)
            .map_err(|error| single(location, format!("cannot read `{path}`: {error}")))
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> PlanError {
    let document = path.display().to_string();
    single(
        SourceLocation::new(document, 1),
        format!("cannot read plan: {error}"),
    )
}

fn single(location: SourceLocation, message: String) -> PlanError {
    PlanError {
        diagnostics: vec![Diagnostic { location, message }],
    }
}

/// Validates a submitted bundle and flattens it into execution order.
///
/// A unit that invokes procedures runs them first, in order, and is then
/// evaluated itself. Units reached through procedures get keys prefixed with
/// the invoking unit, so invoking the same procedure twice yields distinct
/// identities.
///
/// # Errors
///
/// Returns diagnostics for invalid documents, invocations of documents that
/// are missing from the bundle, paths that leave the bundle, external or
/// absolute targets, and recursive invocations.
pub fn expand(bundle: &PlanBundle) -> Result<ExpandedPlan, PlanError> {
    check_paths(bundle)?;
    let entry_source = document(bundle, &bundle.entry, None)?;
    let plan = parse_document(&bundle.entry, entry_source, DocumentRole::Entry)?;
    let mut expander = Expander {
        bundle,
        procedures: BTreeMap::new(),
        units: Vec::new(),
        stack: vec![bundle.entry.clone()],
        diagnostics: Vec::new(),
    };
    expander.plan_units(&plan, None, &[]);
    mark_invoked_cleanup(&mut expander.units);
    if !expander.diagnostics.is_empty() {
        return Err(PlanError {
            diagnostics: expander.diagnostics,
        });
    }
    Ok(ExpandedPlan {
        plan,
        procedures: expander.procedures,
        units: expander.units,
    })
}

/// The units of a procedure that a cleanup unit invokes are cleanup work
/// too, whatever their own titles say.
fn mark_invoked_cleanup(units: &mut [ExecutionUnit]) {
    let cleanup: Vec<UnitKey> = units
        .iter()
        .filter(|unit| unit.identity.cleanup)
        .map(|unit| unit.identity.key.clone())
        .collect();
    for unit in units {
        if unit.invoked_by.iter().any(|key| cleanup.contains(key)) {
            unit.identity.cleanup = true;
        }
    }
}

fn check_paths(bundle: &PlanBundle) -> Result<(), PlanError> {
    let diagnostics: Vec<Diagnostic> = std::iter::once(&bundle.entry)
        .chain(bundle.documents.keys())
        .filter(|path| !is_bundle_path(path))
        .map(|path| Diagnostic {
            location: SourceLocation::new(path.clone(), 1),
            message: "bundle paths must be relative, use `/` and stay inside the bundle".to_owned(),
        })
        .collect();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(PlanError { diagnostics })
    }
}

fn document<'a>(
    bundle: &'a PlanBundle,
    path: &str,
    at: Option<&SourceLocation>,
) -> Result<&'a str, PlanError> {
    bundle
        .documents
        .get(path)
        .map(String::as_str)
        .ok_or_else(|| {
            let location = at.cloned().unwrap_or_else(|| SourceLocation::new(path, 1));
            single(location, format!("`{path}` is not in the bundle"))
        })
}

struct Expander<'a> {
    bundle: &'a PlanBundle,
    procedures: BTreeMap<String, Plan>,
    units: Vec<ExecutionUnit>,
    stack: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

impl Expander<'_> {
    /// Expands `plan`'s units. A nested unit's key names its invoking unit
    /// and `segment`, the procedure document with the invocation's
    /// occurrence when the same unit follows it more than once.
    fn plan_units(
        &mut self,
        plan: &Plan,
        parent: Option<(&UnitKey, &str)>,
        invoked_by: &[UnitKey],
    ) {
        for phase in &plan.phases {
            for unit in &phase.units {
                let key = parent.map_or_else(
                    || UnitKey::new(unit.id.clone()),
                    |(parent, segment)| parent.nested(segment, &unit.id),
                );
                let mut chain = invoked_by.to_vec();
                chain.push(key.clone());
                let mut occurrences = BTreeMap::new();
                for procedure in &unit.procedures {
                    self.invoke(&plan.document, procedure, (&key, &mut occurrences), &chain);
                }
                self.units.push(ExecutionUnit {
                    identity: identity(key, unit, &phase.title),
                    unit: unit.clone(),
                    document: plan.document.clone(),
                    invoked_by: invoked_by.to_vec(),
                });
            }
        }
    }

    fn invoke(
        &mut self,
        from: &str,
        procedure: &ProcedureRef,
        (key, occurrences): (&UnitKey, &mut BTreeMap<String, usize>),
        chain: &[UnitKey],
    ) {
        let parsed = resolve(from, procedure).and_then(|target| self.procedure(&target, procedure));
        match parsed {
            Ok(plan) => {
                let seen = occurrences.entry(plan.document.clone()).or_insert(0);
                *seen += 1;
                let segment = if *seen == 1 {
                    plan.document.clone()
                } else {
                    format!("{}@{seen}", plan.document)
                };
                self.stack.push(plan.document.clone());
                self.plan_units(&plan, Some((key, &segment)), chain);
                self.stack.pop();
            }
            Err(error) => self.diagnostics.extend(error.diagnostics),
        }
    }

    fn procedure(&mut self, target: &str, procedure: &ProcedureRef) -> Result<Plan, PlanError> {
        if self.stack.iter().any(|open| open == target) {
            let chain = format!("{} -> {target}", self.stack.join(" -> "));
            return Err(single(
                procedure.location.clone(),
                format!("recursive procedure invocation: {chain}"),
            ));
        }
        if let Some(plan) = self.procedures.get(target) {
            return Ok(plan.clone());
        }
        let source = document(self.bundle, target, Some(&procedure.location))?;
        let plan = parse_document(target, source, DocumentRole::Procedure)?;
        self.procedures.insert(target.to_owned(), plan.clone());
        Ok(plan)
    }
}

fn identity(key: UnitKey, unit: &Unit, phase: &str) -> UnitIdentity {
    let cleanup = is_cleanup(&unit.title) || is_cleanup(phase);
    UnitIdentity {
        key,
        id: unit.id.clone(),
        kind: unit.kind,
        title: unit.title.clone(),
        phase: phase.to_owned(),
        location: unit.location.clone(),
        cleanup,
        criteria: unit.expected.clone(),
    }
}

/// Whether a unit or phase title marks cleanup work.
fn is_cleanup(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    ["cleanup", "clean up", "clean-up", "teardown", "tear down"]
        .iter()
        .any(|word| lower.contains(word))
}

/// Resolves a procedure link relative to the invoking document into a
/// normalized bundle-relative path.
fn resolve(from: &str, procedure: &ProcedureRef) -> Result<String, PlanError> {
    let fail = |message: &str| {
        let message = format!("procedure `{}`: {message}", procedure.target);
        single(procedure.location.clone(), message)
    };
    check_target(&procedure.target).map_err(fail)?;
    let base = Path::new(from).parent().unwrap_or_else(|| Path::new(""));
    let mut parts: Vec<String> = Vec::new();
    for component in base.join(&procedure.target).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir if parts.pop().is_some() => {}
            Component::CurDir => {}
            _ => return Err(fail("the path leaves the bundle")),
        }
    }
    if parts.is_empty() {
        return Err(fail("the path is empty"));
    }
    Ok(parts.join("/"))
}

/// Rejects link targets that cannot name a document inside the bundle.
fn check_target(target: &str) -> Result<(), &'static str> {
    if target.contains("://") || target.starts_with("mailto:") {
        return Err("external links cannot be invoked; procedures must be documents in the bundle");
    }
    let unsupported = ['\\', '#', '?'];
    if target.starts_with('/') || target.contains(unsupported) {
        return Err("use a relative path to a Markdown file, without anchors");
    }
    Ok(())
}

/// Checks that a bundle-relative path is normalized and stays in the bundle.
#[must_use]
pub fn is_bundle_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests;
