{{- define "beecrawl.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "beecrawl.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name (include "beecrawl.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "beecrawl.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "beecrawl.labels" -}}
helm.sh/chart: {{ include "beecrawl.chart" . }}
app.kubernetes.io/name: {{ include "beecrawl.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
{{- end -}}

{{- define "beecrawl.selectorLabels" -}}
app.kubernetes.io/name: {{ include "beecrawl.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "beecrawl.image" -}}
{{- $image := .image -}}
{{- $tag := default .global.imageTag $image.tag -}}
{{- printf "%s/%s:%s" .global.imageRegistry $image.repository $tag -}}
{{- end -}}

{{- define "beecrawl.rolloutStrategy" -}}
strategy:
  type: RollingUpdate
  rollingUpdate:
    maxSurge: {{ .Values.rollout.maxSurge }}
    maxUnavailable: {{ .Values.rollout.maxUnavailable }}
{{- end -}}
