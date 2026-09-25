# Changes to Liana for the QR bridge, and how to remove them

The QR bridge is a stopgap. This file lists **every change made outside `liana-qr/`** to support it,
so it can be removed cleanly once Liana supports QR code signing devices natively.

The list is taken from `git diff b8d83489 -- . ':!liana-qr'` (upstream `master` when the bridge was
started). Every change sits behind the `qr_bridge` setting, which is off by default: with it off,
Liana behaves exactly as before.

## Summary

| Area | Files | What |
|---|---|---|
| Workspace | `Cargo.toml`, `Cargo.lock` | `liana-qr` crate added to the members |
| Setting | `liana-gui/src/app/settings/mod.rs` | `qr_bridge` field in the global settings file |
| Bridge client | `liana-gui/src/qr_bridge.rs`, `liana-gui/src/lib.rs` | Starts `liana-qr` and reads its answer (the only code talking to the bridge) |
| Settings toggle | `app/state/settings/{general,mod}.rs`, `app/view/settings/general.rs`, `app/view/message.rs` | On/off switch in Settings > General |
| Signing | `app/state/psbt.rs`, `app/view/psbt.rs`, `app/view/message.rs` | "Sign with a QR code device" in the sign dialog |
| Registration, wallet settings | `app/state/settings/wallet.rs`, `app/view/settings/mod.rs`, `app/view/message.rs` | "Register on a QR code device" in Settings > Wallet |
| Registration, installer | `installer/step/descriptor/mod.rs`, `installer/view/mod.rs`, `installer/message.rs` | Same entry in the installer; the step is shown when the setting is on |
| Key import | `installer/step/descriptor/editor/{key,mod}.rs` | "Scan the key of a QR code device" in Set key > Other options |
| Design system | `liana-ui/src/component/{badge,modal/mod}.rs` | `Tile::QrDevice` and two list entry helpers |
| Translations | `liana-i18n/i18n/liana_en.lang` | 5 English strings |

No dependency was added to `liana-gui`, `liana-ui` or `liana-i18n`.

## Removal, step by step

Paths are relative to the repository root. Everything added uses a `qr_bridge`, `QrBridge`, `qr_device`,
`QrDevice`, `QrXpub`, `QrSigner` or `qr-bridge` name, or a comment mentioning the QR bridge, so the
grep in step 5 finds every place. None of these names existed in Liana before.

### 1. The bridge itself

- Delete the `liana-qr/` directory.
- `Cargo.toml`: remove `"liana-qr",` from `[workspace] members`. It is not in `default-members`.
- `Cargo.lock`: nothing to edit by hand. The next `cargo build` drops the bridge's dependencies (`ur`,
  `rqrr`, `nokhwa`, `qrcode`, `base32`, `ciborium`...). None of the existing crates' versions were
  changed when they were added.
- Packaging: if release scripts ship `liana-qr` next to `liana-gui`, remove it from there too. None
  did when this was written.

### 2. `liana-gui`

**`src/qr_bridge.rs`**: delete the file, and its `pub mod qr_bridge;` line in **`src/lib.rs`**.

**`src/app/settings/mod.rs`**, in `pub mod global`:
- Remove the `qr_bridge: Option<bool>` field of `GlobalSettings`, with its doc comment and
  `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Remove `load_qr_bridge` and `update_qr_bridge`.
- In `update()`, remove `&& global_settings.qr_bridge.is_none()` from the "don't create the file" check.
- Settings files already saved with `"qr_bridge": true` still load afterwards: serde ignores unknown
  fields. No migration is needed.

**Settings > General toggle**
- `src/app/state/settings/general.rs`:
  - Remove the `qr_bridge: bool` field of `GeneralSettingsState`.
  - Revert `new(wallet, qr_bridge)` to `new(wallet)`.
  - Remove `self.qr_bridge,` from the `general_section(...)` call in `view()`.
  - Remove the `SettingsMessage::EnableQrBridge(enabled)` arm at the top of `update()`.
- `src/app/state/settings/mod.rs`: in the `SettingsMessage::GeneralSection` arm, revert to
  `general::GeneralSettingsState::new(self.wallet.clone()).into()`.
- `src/app/view/settings/general.rs`:
  - Remove the `qr_bridge: bool` parameter of `general_section`.
  - Remove `.push(qr_bridge_setting(qr_bridge))`.
  - Remove the `qr_bridge_setting` function.

**Messages**, `src/app/view/message.rs`:
- Remove `SelectQrSigner` from `SpendTxMessage`.
- Remove `EnableQrBridge(bool)`, `RegisterOnQrDevice` and `QrDeviceRegistered(Result<bool, String>)`
  from `SettingsMessage`.

**Signing**
- `src/app/state/psbt.rs`:
  - In `PsbtState::update`, remove the `crate::qr_bridge::is_enabled(&cache.datadir_path)` argument of
    `SignModal::new`.
  - In `SignModal`, remove the `qr_bridge` and `qr_signing` fields, the `qr_bridge` parameter of
    `new()`, and their initialisation.
  - In `Modal::update`, remove the `SpendTxMessage::SelectQrSigner` arm, and the
    `self.qr_signing = false;` line at the top of the `Message::Signed` arm.
  - In `view()`, remove the `self.qr_bridge,` argument of `sign_action`.
- `src/app/view/psbt.rs`: in `sign_action`, remove the `qr_bridge: bool` parameter and the
  `if qr_bridge { signers.push(modal::qr_device_entry(...)) }` block.

**Registration from Settings > Wallet**
- `src/app/state/settings/wallet.rs`:
  - In `RegisterWalletModal`, remove the `qr_bridge` field and its initialisation in `new()`.
  - Remove `self.qr_bridge,` from the `register_wallet_modal(...)` call in `view()`.
  - Remove the `SettingsMessage::RegisterOnQrDevice` and `SettingsMessage::QrDeviceRegistered` arms of
    `update()`.
- `src/app/view/settings/mod.rs`: in `register_wallet_modal`, remove the `qr_bridge: bool` parameter
  and the `let signers = signers.push_maybe(qr_bridge.then(...))` block.

**Registration in the installer**
- `src/installer/message.rs`: remove `RegisterOnQrDevice` and `QrDeviceRegistered(Result<bool, String>)`
  from `Message`.
- `src/installer/step/descriptor/mod.rs`, in `RegisterDescriptor`:
  - Remove the `qr_bridge` field and its initialisation in `new()`.
  - Remove the `self.qr_bridge = ...` line in `load_context()`.
  - Remove the `Message::RegisterOnQrDevice` and `Message::QrDeviceRegistered` arms of `update()`.
  - Revert `skip()` to `!ctx.hw_is_used`.
  - Remove `self.qr_bridge,` from the `view::register_descriptor(...)` call in `view()`.
  - **Behaviour to be aware of:** with the setting on, the registration step is shown even when no USB
    device was used, because keys scanned over QR are imported as plain xpubs. Reverting `skip()`
    restores the original behaviour.
- `src/installer/view/mod.rs`: in `register_descriptor`, remove the `qr_bridge: bool` parameter and the
  `let qr_device = qr_bridge.then(...)` block, and revert `column![devices_title, devices, qr_device]`
  to `column![devices_title, devices]`.

**Key import in the installer**
- `src/installer/step/descriptor/editor/mod.rs`, in `DefineDescriptor`:
  - Remove the `qr_bridge` field and its initialisation in `new()`.
  - Remove the `self.qr_bridge,` argument of `SelectKeySource::new(...)`.
  - Remove the `self.qr_bridge = ...` line in `load_context()`.
- `src/installer/step/descriptor/editor/key.rs`:
  - Remove `ScanQrXpub` and `QrXpub(...)` from `SelectKeySourceMessage`.
  - In `SelectKeySource`, remove the `qr_bridge` field and the `qr_bridge` parameter of `new()`.
  - Remove the `on_scan_qr_xpub` and `on_qr_xpub` methods.
  - Remove their two arms in `DescriptorEditModal::update`.
  - In `view_other_options`, remove the `scan_qr_xpub` entry and its `.push_maybe(scan_qr_xpub)`.

### 3. `liana-ui`

- `src/component/badge.rs`: remove the `QrDevice` variant of `Tile`, and its
  `(QrDevice, qr_icon, Neutral, DEFAULT)` line in `tile_specs!`. The `qr_icon` icon itself pre-existed:
  keep it.
- `src/component/modal/mod.rs`: remove `qr_device_entry` and `scan_qr_xpub_entry`.

### 4. `liana-i18n`

Remove these entries from `i18n/liana_en.lang`, then run `cargo run -p liana-i18n-toolbox -- sync`
so the other catalogs follow:

- `qr-bridge-register`
- `qr-bridge-scan-xpub`
- `qr-bridge-sign`
- `settings-qr-bridge`
- `settings-qr-bridge-tooltip`

### 5. Check nothing is left

```sh
git grep -n -i "qr_bridge\|qrbridge\|qr_device\|qrdevice\|qrxpub\|qr_xpub\|qrsigner\|qr_signing\|qr code device\|qr-bridge\|liana-qr" -- . ':!Cargo.lock'
cargo fmt -- --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test
cargo run -p liana-i18n-toolbox -- check-ids
cargo run -p liana-i18n-toolbox -- sync --verify
```

The `git grep` should print nothing. It doesn't match the receive-address QR code Liana had before,
which uses none of these names.

## If native QR support reuses parts of this

- The Liana side already has the shape a native integration needs: an entry in the sign dialog, the
  registration step and the key picker, each ending in the existing merge, registration or xpub path.
  A native version can keep those call sites and replace the `crate::qr_bridge::*` calls with
  in-process code.
- The bridge's `codec/` (UR, BBQr, `pMofN`, key conversion), `psbt.rs` (merging trimmed PSBTs) and
  `device.rs` (per-device formats) have no dependency on the bridge app. They can move into Liana
  with their tests.
