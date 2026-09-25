# Liana QR bridge

Stopgap support for QR code signing devices in Liana, until Liana supports them natively.

The QR code work (camera, animated codes, device formats) lives in this separate app. Liana starts it
for a single action and reads the result back, so users never copy or paste anything. The Liana side
is small, and nothing changes unless the user turns it on.

## For users

1. Turn on **Settings > General > QR code signing devices**. It is off by default.
2. Liana then shows a **QR code device** option:
   - **Sign:** in the transaction's *Sign* dialog, choose *Sign with a QR code device*.
   - **Register the wallet:** in the installer's registration step, or in *Settings > Wallet >
     Register on device*.
   - **Import a key:** when creating a wallet, under *Set key > Other options > Scan the key of a
     QR code device*.
3. The bridge window opens:
   - It shows the QR code to scan with your device, or opens the camera to read the device's answer.
   - The result goes straight back to Liana, and the window closes.
   - *Previous* on the first screen, or closing the window, cancels.

Address verification needs nothing: the devices scan the receive address QR code Liana already shows.

The *Signing device* selector picks the QR format. The bridge remembers it, and you can switch it on
the QR screen if a device can't read the codes.

## Devices

| Device | Transactions | Wallet registration | Notes |
|---|---|---|---|
| Specter DIY | Base64, split in `pMofN` parts | `addwallet <name>&<descriptor>` | Answers in the format it received. The signed PSBT only holds signatures, and the bridge merges them. |
| Krux | UR `crypto-psbt` | Descriptor as UR `bytes` | Lists Liana as a supported coordinator. Answers in the format it received. |
| Coldcard Q | BBQr (`P`) | Descriptor as BBQr (`U`) | Miniscript needs the EDGE firmware. |
| Passport Prime | UR `crypto-psbt` | Descriptor as UR `bytes` | Needs the Liana app. Its QR formats aren't documented yet, so they're unverified. |
| Other (UR / BBQr) | UR or BBQr | UR `bytes` or BBQr `U` | |

Scanning detects UR, BBQr, `pMofN` and single-QR base64/hex automatically, whatever device is selected.
Keys are accepted as text `[fingerprint/path]xpub`, UR `crypto-account`/`crypto-hdkey`, Coldcard JSON
or SLIP-132 (`Zpub`, `Vpub`...), and only for the wallet's network.

**Blockstream Jade is not supported**: it can't register a miniscript descriptor over QR, only
multisig files.

## How Liana talks to the bridge

Liana starts `liana-qr <request>` with piped stdin and stdout, and waits for it to exit. See
`src/protocol.rs` for the bridge side and `liana-gui/src/qr_bridge.rs` for the Liana side.

| Request | stdin | stdout on success |
|---|---|---|
| `sign` | base64 PSBT | the PSBT with the device's signatures merged in |
| `register --name <wallet name>` | descriptor | `registered` |
| `xpub --network <bitcoin\|testnet\|signet\|regtest>` | nothing | `[fingerprint/path]xpub` |

- Empty output means the user cancelled.
- Errors are shown and retried in the bridge, so Liana has nothing to report.
- Liana looks for the `liana-qr` executable next to its own, then on the `PATH`.
- Liana treats the answers like a USB device's:
  - Signatures are merged and saved through the daemon, so it works with local and Liana Connect
    wallets alike.
  - Keys go through the usual xpub checks.

## Removing it

The bridge is meant to go away once Liana supports QR devices natively. See
[LIANA_CHANGES.md](LIANA_CHANGES.md) for every change made to Liana for it, file by file, and how to
remove each one.

## Building

```
cargo build --release -p liana-qr   # then ship target/release/liana-qr next to liana-gui
```

- **Camera:** the `camera` feature (default) uses `nokhwa`: V4L2 on Linux, AVFoundation on macOS
  (the app needs the camera permission), Media Foundation on Windows. Build with
  `--no-default-features` to leave it out; only loading a picture of the device's screen remains.
- **QR codes:** drawn as pixel-exact images, so modules stay crisp for cameras on every renderer.
  Frames change every 250 ms. *QR density* trades the number of frames against how easily a camera
  reads them.

## Layout

- `protocol.rs`: requests from Liana and their answers.
- `codec/`: UR (fountain-coded), BBQr and `pMofN` transports; conversion of scanned keys.
- `psbt.rs`: signature merging.
- `scan.rs`: webcam capture and QR decoding (`rqrr`).
- `device.rs`: per-device formats and hints; the last device used.
- `app.rs`, `view.rs`: the iced app, built from `liana-ui` components (installer layout, list entries,
  buttons, cards).
