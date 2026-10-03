# Unknown monitor refresh rate

Vendored from the published `i-slint-backend-winit` 1.18.1 crate.
Original licensing and source headers are retained in `LICENSES/` and each file.

`frame_throttle.rs` treats a zero monitor refresh rate as unknown, using the
existing 60 Hz fallback. Virtual displays can return `Some(0)` instead of `None`;
dividing by that value panics on the first redraw. This was reproduced by starting
the Windows release under Wine 11.0 on Xvfb without renderer overrides.

Remove this Cargo patch when the upstream backend handles zero refresh rates.
