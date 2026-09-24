# Liana QR bridge

A small companion app to use QR-code signing devices with Liana, until Liana supports them natively.

It is a **stopgap**: it lives in its own crate, talks to Liana only through files and the clipboard,
and changes nothing in `liana-gui`. When native QR support lands, delete this directory and its line in
the workspace `Cargo.toml`.

```
cargo run --release -p liana-qr              # home screen
cargo run --release -p liana-qr -- tx.psbt   # straight to signing this PSBT
```

## What it does

| Flow | In Liana | In the bridge |
|---|---|---|
| **Sign a transaction** | Export the PSBT (file) or copy it. | Shows it as an animated QR code, scans the signed answer back, and merges the signatures into the original PSBT. Save the file and use *Import* on the transaction in Liana. *Sign with another device* chains several QR signers. |
| **Register the wallet** | Copy the descriptor (Settings > Wallet), or export it. | Shows the descriptor in the device's registration format. |
| **Import a key** | Paste the key when creating the wallet. | Scans the device's extended public key (text, UR `crypto-account`/`crypto-hdkey`, Coldcard JSON, SLIP-132 `Zpub`/`Vpub`...) and copies it as `[fingerprint/path]xpub`. |
| **Verify an address** | Show the receive address QR code. | Nothing needed: the devices scan Liana's QR code directly. |

You can also load a picture of the device's screen instead of using the webcam.

## Devices

The *Signing device* selector picks the QR format. You can switch it on the QR screen if a device can't
read the codes.

| Device | Transactions | Wallet registration | Notes |
|---|---|---|---|
| Specter DIY | Base64, split in `pMofN` parts | `addwallet <name>&<descriptor>` | Answers in the format it received. The signed PSBT only holds signatures, and the bridge merges them. |
| Krux | UR `crypto-psbt` | Descriptor as UR `bytes` | Lists Liana as a supported coordinator. Answers in the format it received. |
| Coldcard Q | BBQr (`P`) | Descriptor as BBQr (`U`) | Miniscript needs the EDGE firmware. |
| Passport Prime | UR `crypto-psbt` | Descriptor as UR `bytes` | Needs the Liana app. Its QR formats aren't documented yet, so they're unverified. |
| Other (UR / BBQr) | UR or BBQr | UR `bytes` or BBQr `U` | |

Scanning detects UR, BBQr, `pMofN` and single-QR base64/hex automatically, whatever device is selected.

**Blockstream Jade is not supported**: it can't register a miniscript descriptor over QR, only
multisig files.

## Building

- **Camera:** the `camera` feature (default) uses `nokhwa`: V4L2 on Linux, AVFoundation on macOS
  (the terminal or app bundle needs the camera permission), Media Foundation on Windows. Build with
  `--no-default-features` to leave it out; only picture loading remains.
- **QR codes:** drawn as pixel-exact images rather than with iced's QR widget, so modules stay crisp for
  cameras on every renderer. Frames change every 250 ms. *QR density* trades the number of frames
  against how easily a camera reads them.

## Layout

- `codec/`: UR (fountain-coded), BBQr and `pMofN` transports; conversion of scanned keys.
- `psbt.rs`: signature merging.
- `scan.rs`: webcam capture and QR decoding (`rqrr`).
- `device.rs`: per-device formats and hints.
- `app.rs`, `view.rs`: the iced app, built from `liana-ui` components (installer layout, list entries,
  buttons, cards). Its few specific widgets stay in `view.rs` so removing the crate leaves nothing behind
  in `liana-ui`.
