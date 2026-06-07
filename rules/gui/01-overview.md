# iris-gui — overview

Optional egui-based launcher for iris. Lives as a separate workspace crate
(`iris-gui/`); default `cargo build` does not include it. Build with
`cargo build -p iris-gui --release`.

## Process / thread model

- The eframe app owns the **single** `winit::EventLoop` in the process. iris's
  own `src/ui.rs` event loop is **not** used by iris-gui — the GUI starts the
  emulator in headless mode and (Phase B) renders the REX3 framebuffer into
  an egui panel itself.
- A worker thread (`iris-gui/src/handle.rs::worker_loop`) owns the `Machine`.
  GUI ↔ worker communication is via `crossbeam_channel` (`Cmd` and `Evt`).
  The worker thread has an 8 MB stack to satisfy `Machine::new`'s
  `Physical::device_map` allocation, matching `src/main.rs`.
- Settings (recents, window size, ui scale, fullscreen) persist to
  `~/.config/iris/gui.json`. Machine configs stay in TOML so they remain
  runnable via `iris --config …`.

## Safe-stop logic (`src/safe_stop.rs`)

Stopping is "safe" iff any of:
1. PowerOff event observed (IRIX `halt` completed).
2. CPU is sitting at the PROM monitor.
3. Zero dirty COW overlay sectors and no in-flight SCSI writes.

Otherwise a modal lists the failing condition(s), plus a per-CHD warning when
a SCSI device uses a `.chd` image without `overlay = true` (writes are lost).
Modal offers **Cancel / Send IRIX halt / Force stop**.

## What the GUI knows about iris

Only the public API: `MachineConfig`, `Cli`, `Machine::{new, start, stop,
register_system_controller}`. Anything else the GUI needs (status query,
event subscription, framebuffer access) is added as a `pub fn` accessor on
the existing type, never by reaching into private fields.

## Empty-media CD-ROM (Phase B item #6, landed)

`ScsiDevice.backend` is `Option<DiskBackend>`. `None` represents "drive
present, tray empty": INQUIRY still answers, TEST UNIT READY / READ
CAPACITY / READ / READ TOC return `CHECK CONDITION` with sense key
`0x02` (NOT_READY) + ASC `0x3A` (MEDIUM NOT PRESENT). Construct via
`ScsiDevice::new_empty_cdrom()`; mount/swap media with
`Wd33c93a::insert_disc(id, path)`; unload with
`Wd33c93a::eject_to_empty(id)`. In `iris.toml` an empty-tray CD-ROM is
`cdrom = true` with an empty `path` and no `discs` — `MachineConfig::
validate` accepts this state.

## Phase B — embedded framebuffer & input (landed)

The GUI installs an `iris::rex3::Renderer` impl
(`iris-gui/src/framebuffer.rs::CaptureRenderer`) in
`Rex3::renderer` immediately after `Machine::new`, before the CPU
starts. Each `render(buffer, width, height)` call from the REX3 refresh
thread does a stride-aware copy of `width × height` u32 pixels into a
`FrameSink` (parking_lot Mutex of `Frame { width, height, rgba, seq }`).
The main thread reads the sink each egui frame, uploads to a lazily
allocated `egui::TextureHandle`, and renders centered in the central
panel with aspect-preserving fit.

PS/2 input flows through `iris-gui/src/input.rs::pump`. Modifiers
(shift/ctrl/alt/super) are diffed against the previous frame and
synthesised as `ShiftLeft / ControlLeft / AltLeft / SuperLeft`
press/release events because egui delivers modifiers as a separate
field, not as `Key` events. egui `Key` → `winit::keyboard::KeyCode`
mapping covers letters/digits/punctuation/F-keys/navigation; misses
return `None` and are dropped. Mouse events fire only when the cursor
is inside the framebuffer rect — menu / config clicks don't leak into
the guest. F11 is consumed by the GUI (fullscreen toggle) and never
forwarded.

`Cmd::SaveState` calls `Machine::save_snapshot` then `Machine::start`
(save_snapshot stops the CPU as part of its work). `Cmd::RestoreState`
calls `Machine::ci_restore`. `Cmd::Screenshot` PNG-encodes the latest
`FrameSink` snapshot via the `png` crate.

The safe-stop "Send IRIX halt" button TCP-connects to
`127.0.0.1:8881` (iris's standing ttyd1 listener in non-CI mode) and
writes `halt\n`.

## Phase B follow-ups

- Embed REX3 framebuffer into an egui panel (add `Rex3::snapshot_rgba()` or
  similar, upload to egui texture each frame).
- Wire egui key/pointer events → `Ps2Controller` input.
- Status polling: add `Machine::is_in_prom()`, `Machine::dirty_cow_sectors()`,
  `Machine::subscribe_events() -> Receiver<MachineEvent>`.
- Hook `Machine::ci_save` / `ci_restore` / screenshot to the existing Cmd
  variants — currently they `Evt::Error` "not yet wired".
- "Send IRIX halt" should write `halt\n` via the existing serial-send CI
  path (currently falls through to force-stop with a toast).
- Replace text-field path entry with `rfd` pickers throughout the config UI
  (PROM, NVRAM, SCSI image paths, NFS dir).
