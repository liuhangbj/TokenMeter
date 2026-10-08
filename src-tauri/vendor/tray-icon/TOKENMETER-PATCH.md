# TokenMeter compatibility backport

- Source: crates.io `tray-icon` 0.24.2, upstream commit
  `e05dab06db5b441efe5c26f9be2e0c29f1ba2089`.
- Original crate checksum (SHA256):
  `045979e3f037cd18ad1cb2a419dfda133c5c29c9f3453370079f2255d46c257e`.
- Bug: https://github.com/tauri-apps/tray-icon/issues/355
- Backported source: https://github.com/tauri-apps/tray-icon/pull/365
  at `5750c67d02d32f2af100e120ad665959d4268cef` (not yet merged when imported).
- Original fix: https://github.com/tauri-apps/tray-icon/pull/341
- License: MIT OR Apache-2.0; original license files are preserved.

Only `src/platform_impl/macos/mod.rs` is modified. macOS 27 intercepts left clicks
when an NSMenu is attached to NSStatusItem. Retain the menu separately, attach it
only during presentation, and detach it afterwards. Release the macOS menu ivar's
RefCell borrow before entering the native nested menu event loop, including the
internal `show_menu` implementation. This does not change the public wrapper's
outer `self.tray` borrow or its pre-existing programmatic reentrancy limitations;
TokenMeter does not call that public `show_menu` path.
Windows, Linux, public APIs and dependency requirements remain unchanged.

Imported package build-cache metadata and its standalone Cargo.lock are omitted;
TokenMeter's Cargo.lock is authoritative. No global Cargo registry files are edited.

After an upstream release includes this fix, remove the path patch and this vendor
directory only after macOS 27 and the oldest supported macOS tray checks pass:
left open/close, right menu/cancel/action, left-after-right, fast repeated clicks,
panel focus/position, and orb entry. Do not replace it with a moving Git branch.
