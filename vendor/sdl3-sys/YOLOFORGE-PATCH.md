# SDL3 Wayland file-drop correction

This is sdl3-sys 0.6.8+SDL-3.4.14, vendored from crates.io (original VCS commit
`42857dd9f189db290039460a8887f4fcaa43041f`). Upstream license notices are retained.
The Rust bindings are unchanged. Only build.rs and build-common.rs are altered.

The SDL3 Wayland receiver first accepts `text/uri-list`, but then overwrites
the chosen MIME with `text/plain` if both are offered. It keeps `has_mime_file`
and parses the plain text as URIs, losing file-manager drops on release.

The build now copies the registry-pinned SDL 3.4.14 sources to its private
OUT_DIR and applies one guarded fix: skip plain-text negotiation when a file
MIME was already selected. Cargo's shared registry cache is never modified.
No extra Git checkout, external patch command or system SDL install is needed.
The normal text clipboard remains unchanged. The patch fails closed if its
expected source fragment does not match exactly once during a future upgrade.

`scripts/smoke-gui-wayland.py` reproduces the regression using a real GTK drag
offering both file URIs and plain text; `--uri-only` checks the baseline and
`--thunar` uses an isolated real file-manager instance. No Factory services run.
Remove this override once the pinned SDL release contains the upstream fix.
