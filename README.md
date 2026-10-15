# NFCoffeeRst (nfcoffee)

A **portable, mouse-enabled terminal (TUI) client** in Rust for the
`NFC_SD_WIFI` coffee reader. It is a re-implementation of the Android `NFCaffe`
app: pull the coffee counters from the device over WiFi, manage the users/cards,
and import/export the data.

* Runs on **Linux** (dev/test) and **Windows** (portable `.exe`, no admin).
* Single binary, **bundled SQLite** — no runtime to install.
* Data lives **next to the executable** (`NFCoffeeData/`), so it is fully
  portable (USB stick friendly). Override with `NFCOFFEE_DATA_DIR`.
* Keyboard **and mouse** support (click tabs/buttons/rows, wheel scrolling).

See [`REQUIREMENTS.md`](REQUIREMENTS.md) for the reverse-engineered device
protocol and the full design notes.

---

## Quick start (Linux)

```bash
cargo run --release
```

## Quick start (Windows)

A prebuilt `dist/nfcoffee.exe` is produced by the cross-build (see below).
Just copy it anywhere and run it from a terminal (PowerShell / cmd):

```powershell
.\nfcoffee.exe
```

### Building for Windows

**Option A — build on Windows (recommended, simplest):**

1. Install Rust (rustup) — the default MSVC toolchain also needs the
   *Build Tools for Visual Studio* (C++ workload) for the bundled SQLite.
   Alternatively install the GNU toolchain (rustup + mingw-w64).
2. Build a fully self-contained exe:
   ```powershell
   $env:RUSTFLAGS="-C target-feature=+crt-static"
   cargo build --release
   ```
   Result: `target\release\nfcoffee.exe` — copy it anywhere.

**Option B — cross-compile from Linux:**

```bash
rustup target add x86_64-pc-windows-gnullvm
# download and extract https://github.com/mstorsjo/llvm-mingw/releases
MINGW=/path/to/llvm-mingw/bin ./build-windows.sh
```

This produces a self-contained `dist/nfcoffee.exe` that imports only Windows
system DLLs (verified). See `build-windows.sh` for the details.

---

## Using it

1. Present the **master card** to the reader — the device turns on its WiFi AP
   (`NFCoffee`) and waits for a client.
2. In the app, press **T** (or click *Transfer*). The app tries to join the AP
   (via a pre-connect command, or `nmcli`/`netsh`) and streams the counters.
3. Counts received from the device are **added** to the stored totals, exactly
   like the Android app. The device then archives its counter files.

### Configuring WiFi

`Settings` (F4):

| Field | Meaning |
|---|---|
| Device IP / TCP port | `192.168.4.1` / `31415` by default |
| AP SSID / password | `NFCoffee` / `freecoffee` |
| Pre-connect command | optional; run before each transfer (`sh -c` / `cmd /C`) |
| Auto-connect | use `nmcli` (Linux) / `netsh` (Windows) to join the AP |
| Zero-based month | keep `true` for byte-identical backup folder names with Android |
| Connect / transfer timeout | seconds |

If automatic joining is blocked by the OS, connect to the `NFCoffee` network
manually and press Transfer again.

### Importing your existing data

The Android export format is supported unchanged:

```
NFCID,CODENAME,UNAME,EMAIL,COFF
BB36AB22,00001FY0.V1E,Alice,alice@example.com,4
```

Press **I** (Import) and enter the CSV path. Import *adds* the CSV counts to the
stored values (same behaviour as the old app). **E** exports the same format.

> Tip: to migrate the whole Android database you can also drop the old
> `NFCoffee` SQLite file into `NFCoffeeData/` as `nfcoffee.db` — the schema is
> identical (`NFCIDTable`).

### Key bindings

| Key | Action |
|---|---|
| F1 | Help |
| F2 / F3 / F4 | Dashboard / Users / Settings |
| Ctrl+Q / Ctrl+C | Quit |
| Dashboard: **T**/Enter | Transfer data |
| Dashboard: **C** / **E** / **I** | Clear counters / Export / Import |
| Users: ↑↓, Enter | Select / Edit |
| Users: **N** / **D** / **C** | New / Delete / Clear counters |
| Users: **/** | Search filter |
| Forms: Tab/Enter next, Ctrl+S/Enter(save), Esc cancel | |
| Mouse | click tabs, buttons, rows; wheel scrolls |

---

## Data & files

```
<folder of nfcoffee[.exe]>/
└── NFCoffeeData/
    ├── nfcoffee.db     # SQLite, table NFCIDTable (Android schema)
    ├── settings.json   # connection settings
    └── exports/        # default CSV export location
```

Set `NFCOFFEE_DATA_DIR` to put this elsewhere (e.g. a shared folder).

---

## Development

```bash
cargo test        # protocol, DB merge/import/export and TUI render tests
cargo build --release
```

Tested with Rust 1.96. The included tests cover:

* base36 CODENAME encode/decode and the handshake key layout,
* the full TCP handshake against a mock device socket,
* transfer merge / CSV import / export against SQLite,
* rendering of every screen and overlay via ratatui's `TestBackend`.
