# ADR-001: Split-architecture runtime

Status: Draft

Context:
- We decouple fieldbus I/O from logic execution.

Decision:
- Separate real-time fieldbus plane from logic plane.

Consequences:
- Better fault isolation and RT determinism.
