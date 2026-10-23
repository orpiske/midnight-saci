//! `saci skill`: lists the agent skills embedded in the CLI and installs them
//! where a coding agent finds them.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// One embedded skill: a directory with a `SKILL.md` and optional files.
pub struct Skill {
    /// Directory name, which agents use as the skill's name.
    pub name: &'static str,
    /// Files relative to the skill directory, with their contents.
    pub files: &'static [(&'static str, &'static str)],
}

/// The skills shipped with Saci. Each one lives in `skills/<name>/` at the
/// repository root; add new ones here. Names start with `saci-` so they do
/// not clash with other skills in the same agent directory.
pub const SKILLS: &[Skill] = &[
    Skill {
        name: "saci-convert-test-plan",
        files: &[(
            "SKILL.md",
            include_str!("../../../skills/saci-convert-test-plan/SKILL.md"),
        )],
    },
    Skill {
        name: "saci-investigate",
        files: &[(
            "SKILL.md",
            include_str!("../../../skills/saci-investigate/SKILL.md"),
        )],
    },
    Skill {
        name: "saci-run-test-plan",
        files: &[(
            "SKILL.md",
            include_str!("../../../skills/saci-run-test-plan/SKILL.md"),
        )],
    },
    Skill {
        name: "saci-start-backend",
        files: &[
            (
                "SKILL.md",
                include_str!("../../../skills/saci-start-backend/SKILL.md"),
            ),
            (
                "ensure-backend.sh",
                include_str!("../../../skills/saci-start-backend/ensure-backend.sh"),
            ),
        ],
    },
];

/// Coding agents and the directories where they look for skills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    /// Claude Code: `.claude/skills`, `~/.claude/skills`.
    Claude,
    /// OpenAI Codex: `.agents/skills`, `~/.agents/skills`.
    Codex,
    /// Cursor: `.cursor/skills`, `~/.cursor/skills`.
    Cursor,
    /// GitHub Copilot: `.github/skills`, `~/.copilot/skills`.
    Copilot,
    /// Gemini CLI: `.gemini/skills`, `~/.gemini/skills`.
    Gemini,
    /// OpenCode: `.opencode/skills`, `~/.config/opencode/skills`.
    Opencode,
    /// The shared `.agents/skills` and `~/.agents/skills` that most agents
    /// other than Claude Code read.
    Agents,
}

impl Agent {
    /// The skills directory, relative to the project or, with `global`, to
    /// the home directory.
    const fn directory(self, global: bool) -> &'static str {
        match (self, global) {
            (Self::Claude, _) => ".claude/skills",
            (Self::Codex | Self::Agents, _) => ".agents/skills",
            (Self::Cursor, _) => ".cursor/skills",
            (Self::Copilot, false) => ".github/skills",
            (Self::Copilot, true) => ".copilot/skills",
            (Self::Gemini, _) => ".gemini/skills",
            (Self::Opencode, false) => ".opencode/skills",
            (Self::Opencode, true) => ".config/opencode/skills",
        }
    }
}

/// `saci skill` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum SkillCommand {
    /// List the skills that ship with Saci.
    List,
    /// Install skills where a coding agent finds them.
    ///
    /// Without --global, installs into the project in the current directory,
    /// for example .claude/skills/<name>/ for --agent claude. An installed
    /// skill that differs from this version is not replaced without --force.
    Install {
        /// Skills to install; see `saci skill list`.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        names: Vec<String>,
        /// Install every skill.
        #[arg(long)]
        all: bool,
        /// Agent to install for. Repeat it for several agents.
        #[arg(long, short, required_unless_present = "dir")]
        agent: Vec<Agent>,
        /// Install into the agent's directory in your home directory.
        #[arg(long, conflicts_with = "dir")]
        global: bool,
        /// Install into this skills directory instead, for any other agent.
        #[arg(long, value_name = "DIR", conflicts_with = "agent")]
        dir: Option<PathBuf>,
        /// Replace installed skills that differ from this version.
        #[arg(long)]
        force: bool,
    },
}

/// Runs a `saci skill` subcommand.
///
/// # Errors
///
/// Fails for unknown skill names, a missing home directory with --global,
/// installed skills that differ without --force, and write errors.
pub fn run(command: SkillCommand) -> anyhow::Result<()> {
    match command {
        SkillCommand::List => {
            for skill in SKILLS {
                println!("{}\t{}", skill.name, description(skill));
            }
            Ok(())
        }
        SkillCommand::Install {
            names,
            all,
            agent,
            global,
            dir,
            force,
        } => {
            let skills = select(&names, all)?;
            for target in targets(&agent, global, dir)? {
                for skill in &skills {
                    install(skill, &target, force)?;
                }
            }
            Ok(())
        }
    }
}

/// The `description` from the skill's front matter.
fn description(skill: &Skill) -> &'static str {
    skill
        .files
        .iter()
        .find(|(path, _)| *path == "SKILL.md")
        .and_then(|(_, text)| {
            text.lines()
                .take_while(|line| !line.is_empty())
                .find_map(|line| line.strip_prefix("description:"))
        })
        .map_or("", str::trim)
}

fn select(names: &[String], all: bool) -> anyhow::Result<Vec<&'static Skill>> {
    if all {
        return Ok(SKILLS.iter().collect());
    }
    names
        .iter()
        .map(|name| {
            SKILLS
                .iter()
                .find(|skill| skill.name == name)
                .with_context(|| {
                    let known: Vec<_> = SKILLS.iter().map(|skill| skill.name).collect();
                    format!("unknown skill `{name}`; available: {}", known.join(", "))
                })
        })
        .collect()
}

/// The skills directories to install into, without duplicates.
fn targets(agents: &[Agent], global: bool, dir: Option<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    if let Some(dir) = dir {
        return Ok(vec![dir]);
    }
    let base = if global {
        std::env::home_dir().context("cannot find your home directory for --global")?
    } else {
        PathBuf::new()
    };
    let mut targets: Vec<PathBuf> = Vec::new();
    for agent in agents {
        let target = base.join(agent.directory(global));
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    Ok(targets)
}

/// Writes `skill` into `<target>/<name>/`.
fn install(skill: &Skill, target: &Path, force: bool) -> anyhow::Result<()> {
    let root = target.join(skill.name);
    let installed = skill
        .files
        .iter()
        .map(|(path, text)| std::fs::read_to_string(root.join(path)).is_ok_and(|old| old == *text))
        .collect::<Vec<_>>();
    if installed.iter().all(|same| *same) {
        println!("{} is up to date in {}", skill.name, root.display());
        return Ok(());
    }
    if root.exists() && !force {
        bail!(
            "{} exists and differs from this version; use --force to replace it",
            root.display()
        );
    }
    for (path, text) in skill.files {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        std::fs::write(&file, text).with_context(|| format!("cannot write {}", file.display()))?;
    }
    println!("Installed {} in {}", skill.name, root.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn every_skill_directory_and_file_is_embedded() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        let mut embedded: Vec<_> = SKILLS.iter().map(|skill| skill.name.to_owned()).collect();
        embedded.sort();
        assert_eq!(sorted_entries(&root), embedded, "add new skills to SKILLS");
        for skill in SKILLS {
            let mut listed: Vec<_> = skill
                .files
                .iter()
                .map(|(path, _)| (*path).to_owned())
                .collect();
            listed.sort();
            assert_eq!(
                sorted_entries(&root.join(skill.name)),
                listed,
                "add every file of {} to SKILLS",
                skill.name
            );
        }
    }

    #[test]
    fn every_skill_has_a_prefixed_name_and_a_description() {
        for skill in SKILLS {
            assert!(
                skill.name.starts_with("saci-"),
                "{} needs the saci- prefix",
                skill.name
            );
            let front_matter = format!("---\nname: {}\n", skill.name);
            assert!(
                skill
                    .files
                    .iter()
                    .any(|(path, text)| *path == "SKILL.md" && text.starts_with(&front_matter)),
                "the SKILL.md name of {} must match its directory",
                skill.name
            );
            assert!(
                !description(skill).is_empty(),
                "{} has no description",
                skill.name
            );
        }
    }
}
