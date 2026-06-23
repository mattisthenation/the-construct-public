# Kubernetes notes

A few things I keep forgetting:

- A Deployment manages a ReplicaSet, which manages Pods.
- `kubectl rollout undo` reverts to the previous revision.
- Liveness probes restart a container; readiness probes pull it out of the Service endpoints.
- ConfigMaps and Secrets both mount as volumes or env vars — Secrets are just base64, not encrypted at rest by default.

#theconstruct/file-this
