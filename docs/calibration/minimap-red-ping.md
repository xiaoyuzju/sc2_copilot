# Minimap red-ping calibration log

This log stores only aggregated evidence collected independently by SC2 Copilot. It does not retain
raw endpoint responses, gameplay images, player or account identifiers, local filesystem paths, or
machine-identifying metadata.

## 2026-07-25 — Void Rifts, no ping in target region

- Source: live cooperative match observed through the local SC2 `6119` API and the project DXGI
  capture adapter.
- Environment: Windows 11, Chinese SC2 client, 3840×2160 HDR desktop, strict 16:9 game client.
- Map/session: normalized map id `void-rifts`, fresh ephemeral local session.
- Observation window: game time 03:00–03:10.
- Capture result: 47 valid normalized 264×259 minimap frames; the first transition frame was
  rejected as `InvalidMinimap` and did not count as absence evidence.
- Detector result: no `Candidate` or `Confirmed` red ping in the target region.
- Policy result: `layout-b`, emitted only after the window ended.
- Command:

  ```text
  cargo test -p sc2-copilot-app \
    capture::tests::resolves_a_live_target_map_variant_end_to_end \
    --lib -- --ignored --nocapture
  ```

This run independently validates the live capture path, unavailable-frame handling, absence
semantics, Void Rifts time window, and `layout-b` mapping. A real `Confirmed` target-region ping and
the Temple of the Past mapping remain explicit follow-up calibration cases; their deterministic
geometry and policy paths are covered by generated ROI and integration tests.
