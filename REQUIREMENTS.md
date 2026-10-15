# NFCoffeeRst — Requirements & Design

Portable, mouse-enabled **TUI** client (in Rust) that replaces the Android
`NFCaffe` app for managing the `NFC_SD_WIFI` coffee-reader device.

- Target 1 (primary): **Windows, portable, no admin rights** — single `.exe`,
  no installer, data stored next to the executable.
- Target 2 (dev/test): **Linux** (where the author tests).
- UI: terminal UI (TUI) with keyboard **and mouse** support (ratatui + crossterm).
- Storage: **SQLite** (bundled), schema identical to the Android app.
- Import/Export: **Android CSV format**, unchanged.

---

## 1. Reverse-engineered protocol (firmware `NFC_SD_WIFI`)

### 1.1 Device / WiFi
| Item | Value | Source |
|---|---|---|
| SSID | `NFCoffee` | `Utils.cpp` (`ssid`) |
| Password | `freecoffee` | `Utils.cpp` (`password`) |
| AP gateway IP | `192.168.4.1` (ESP8266 softAP default) | standard ESP8266 |
| TCP port | `31415` | `Utils.cpp` `WiFiServer server(31415)` |

The device enters transfer mode when a **master card** is presented
(`0x000000BB36AB22` or `0x000000651B121C`). It turns on the AP, starts the TCP
server, waits for a client, receives the key, streams the data, then moves its
card files into `/YYYYMMDD.BKP` on the SD card (see §1.4).

### 1.2 Handshake — 8-byte secret key (client → device)
From `WLAN::ReadSecretKey()` (firmware) and `getTCPData.run()` (Android):

| byte | meaning |
|---|---|
| 0 | checksum high |
| 1 | checksum low |
| 2 | year high |
| 3 | year low |
| 4 | month |
| 5 | day |
| 6 | checksum low |
| 7 | checksum high |

`checksum = year + month + day`.
Device accepts when `chk1 == chk2` **and** `year+month+day == chk1`, then builds
the backup folder name `sprintf("%04d%02d%02d", year, month, day)`.

> **Compatibility quirk:** the Android app sends `Calendar.MONTH` which is
> **0-based** (Jan = 0). To keep the backup folder names byte-identical with the
> old client, this app does the same by default (configurable:
> `zero_based_month`).

### 1.3 Data stream (device → client)
`Utils::printDirectory()` sends, for every non-directory file in the SD root:

```
<CODENAME>,<coffees>\r\n      (println -> CRLF)
```

`CODENAME` is the card UID encoded in **base36**, formatted as
`IIIIIIII.FFF` (11 base36 digits, a `.` before the last three). The UID→number
mapping is little-endian over the UID bytes (firmware union `kUser.ID.u64`),
matching Android `EditID.ByteArrayToHexString`.

* Encode: `base36(u64)`, left-padded with `0` to 11 chars, insert `.` before the
  last 3 chars.
* Decode: drop `.`, parse base36 → u64 → uppercase hex (no leading zeros).

The transfer ends when the device closes the socket.

### 1.4 Counter file format on the SD card (FYI)
Each card file holds 4 bytes: `[count_hi, count_lo, ~count_hi, ~count_lo]`.
The device resets the counters by moving files into `/<date>.BKP` after a
successful transfer, so each transfer only carries *new* coffees.

### 1.5 Client merge rule (must match Android `TCPTransfer` / `ShareData`)
For every received `(CODENAME, count)`:
`new_coff = existing_coff + count` (insert row if missing).

---

## 2. Features (parity with the Android app)

1. **Transfer data** — connect to the device AP, handshake, stream, merge into DB.
2. **Manage users/IDs** — list (NFCID, CODENAME, NAME, EMAIL, COFF), edit, add,
   delete, **clear all counters**.
3. **Manual new-ID entry** — type a UID (hex) or a CODENAME; the app derives the
   other automatically. (Replaces the phone's NFC hardware.)
4. **Import / Export CSV** — Android format:
   `NFCID,CODENAME,UNAME,EMAIL,COFF` (see §3).
5. **Settings** — host, port, SSID, password, pre-connect command, auto-connect,
   month mode, timeouts. Persisted to `settings.json`.

### 2.1 Deliberately deferred (roadmap)
* **PC/SC USB NFC reader** support (read a card on the desktop). The code is
  structured so this can be added as an optional feature flag later, without
  affecting the portable single-binary build.

---

## 3. CSV format (Android compatible, unchanged)
```
NFCID,CODENAME,UNAME,EMAIL,COFF
```
* Rows sorted by `UNAME` ascending (as Android did).
* Import behaviour matches Android: header line is ignored/validated, then for
  each row `existing_coff + row_coff` is stored.
* Fields are comma-separated **without quoting** (exactly like the Android app),
  so names must not contain commas.

---

## 4. WiFi / connectivity strategy

A desktop app cannot join WiFi as transparently as Android. Strategy:

1. If **pre-connect command** is configured in Settings → run it (via
   `sh -c` / `cmd /C`). Maximum flexibility.
2. Else if **auto-connect** is enabled → best-effort platform integration:
   * **Linux:** `nmcli device wifi connect <SSID> password <PASS>`
     (after checking the AP is visible with `nmcli device wifi list`).
   * **Windows:** `netsh wlan show networks` to detect, ensure a profile exists
     (`netsh wlan add profile ... user=current`), then `netsh wlan connect`.
3. Either way, then poll `host:port` until the device answers (timeout from
   settings). If it never answers, show clear manual instructions.
4. The app never requires admin for its own operation; if the OS blocks the
   attempt, the user connects manually and presses Transfer again.

---

## 5. Data location (portable)
`<dir of executable>/NFCoffeeData/` containing:
* `nfcoffee.db` — SQLite (Android schema `NFCIDTable`)
* `settings.json`
* `exports/` — default CSV export target

Override with env var `NFCOFFEE_DATA_DIR`. This keeps the app fully portable and
writable without admin rights (falls back to CWD if the exe dir is not writable).

---

## 6. Key bindings (summary)
| Key | Action |
|---|---|
| F1 | Help |
| F2 / F3 / F4 | Dashboard / Users / Settings |
| Ctrl+C, Ctrl+Q | Quit |
| Dashboard: `t`/Enter | Transfer data |
| Dashboard: `e` / `i` | Export / Import CSV |
| Dashboard: `c` | Clear counters |
| Users: ↑/↓, Enter | Select / Edit |
| Users: `n` `d` | New / Delete |
| Users: `/` | Search |
| Edit/Settings: Tab/Enter next, Ctrl+S save, Esc cancel | |
| Mouse | click tabs, buttons, rows; wheel scrolls |
