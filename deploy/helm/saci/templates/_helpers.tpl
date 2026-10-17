{{- define "saci.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "saci.labels" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "saci.selector" -}}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: backend
{{- end -}}

{{- define "saci.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) -}}
{{- end -}}

{{- define "saci.backendServiceAccount" -}}
{{- default (include "saci.fullname" .) .Values.backend.serviceAccount.name -}}
{{- end -}}

{{- define "saci.workerServiceAccount" -}}
{{- default (printf "%s-worker" (include "saci.fullname" .)) .Values.worker.serviceAccount.name -}}
{{- end -}}

{{- define "saci.authSecret" -}}
{{- default (printf "%s-auth" (include "saci.fullname" .)) .Values.auth.existingSecret -}}
{{- end -}}

{{- define "saci.claim" -}}
{{- default (printf "%s-data" (include "saci.fullname" .)) .Values.persistence.existingClaim -}}
{{- end -}}

{{/* saci.yaml, rendered from the values. */}}
{{- define "saci.config" -}}
{{- $worker := dict "runtime" "kubernetes" "image" (default (include "saci.image" .) .Values.worker.image) -}}
{{- $kubernetes := dict "namespace" .Release.Namespace "image_pull_policy" .Values.image.pullPolicy -}}
{{- if or .Values.worker.serviceAccount.create .Values.worker.serviceAccount.name -}}
{{- $_ := set $kubernetes "service_account" (include "saci.workerServiceAccount" .) -}}
{{- end -}}
{{- $_ := set $worker "kubernetes" $kubernetes -}}
{{- $backend := dict "listen" (printf "0.0.0.0:%v" .Values.service.port) "data_dir" "/var/lib/saci" -}}
{{- $_ := set $backend "worker_url" (printf "http://%s.%s.svc:%v" (include "saci.fullname" .) .Release.Namespace .Values.service.port) -}}
{{- $_ := set $backend "auth" (dict "token_env" "SACI_API_TOKEN") -}}
{{- $llm := dict "base_url" .Values.llm.baseUrl "model" .Values.llm.model -}}
{{- if .Values.llm.apiKey.existingSecret -}}
{{- $_ := set $llm "api_key_env" "SACI_LLM_API_KEY" -}}
{{- $_ := set $llm "api_key_secret" (dict "name" .Values.llm.apiKey.existingSecret "key" .Values.llm.apiKey.key) -}}
{{- end -}}
{{- $config := dict "backend" $backend "worker" $worker "llm" $llm "environments" .Values.config.environments -}}
{{- range $key := list "limits" "policies" "mcp_servers" "actions" -}}
{{- with index $.Values.config $key -}}
{{- $_ := set $config $key . -}}
{{- end -}}
{{- end -}}
{{- toYaml $config -}}
{{- end -}}
