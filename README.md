# El Niño Monitor 2026–27

Bilingual (ES/EN) public dashboard for the 2026–27 El Niño: ocean state, official
indices and forecasts, Peru (ENFEN / ICEN) and worldwide impacts. Every number shows
its source and date.

- **Backend:** Rust (Axum + Tokio), SQLite (WAL), minijinja templates. ~20 MB RAM.
- **Frontend:** server-rendered HTML, vanilla ES modules, ECharts and MapLibre GL
  (self-hosted under `static/vendor`; no build step, no CDN, no API keys).
- **Deploy:** a single Docker container behind Caddy on the host.

The **Learn** page (`/es/learn`, `/en/learn`) is an interactive explainer: nine lessons
(the tropical Pacific, trade winds and upwelling, how an event starts, the three phases,
Peru's coastal El Niño, how it is measured, global impacts, history, and the current
event). Each lesson drives a hand-drawn isometric SVG model with sliders and
animations, and the last lessons use the live data from the database.

## Data sources

| Source | Data | Poll |
|---|---|---|
| NOAA CPC | Weekly Niño 1+2/3/3.4/4 (`wksst9120.for`), ONI, RONI, SOI | 6–12 h |
| NOAA CPC | ENSO Diagnostic Discussion + official RONI outlook, probabilities, strength tables | 6 h |
| NOAA PSL | MEI.v2 | 12 h |
| NOAA NCEI | Daily OISST v2.1 (preliminary) anomaly via CoastWatch ERDDAP, 1° | 6 h |
| NOAA NCEI | Global land+ocean temperature anomaly (Climate at a Glance) | 12 h |
| UCSB CHC | Daily precipitation (CHIRPS v3.0 preliminary), 0.05°, Peru departments | 12 h |
| SENAMHI | Daily rain-gauge precipitation, ~310 real-time conventional + ~135 automatic stations → department means and wettest station | 24 h |
| IGP / ENFEN | ICEN (Índice Costero El Niño) | 12 h |
| ENFEN | Comunicados oficiales (WordPress API + homepage PDF list) | 6 h |
| GDACS | Orange/red flood, drought, cyclone and wildfire alerts | 30 min |

Requests are conditional (ETag / Last-Modified + content hash), so unchanged files
cost one round trip. Every payload that changes is archived in `DATA_DIR/raw/` for provenance.
Status of each source is visible at `/es/sources`.

Curated low-frequency links (WMO, BoM, JMA, IRI, C3S, …) live in the `link` table:

```sh
elnino add-link WMO "El Niño/La Niña Update — Nov 2026" https://wmo.int/... 2026-11-05
```

## Run locally

```sh
cargo test                               # parsers are tested against tests/fixtures
DATA_DIR=./data CONTACT=you@example.com cargo run --release -- backfill   # ~2 min, fills 90 days of SST
DATA_DIR=./data cargo run --release      # http://127.0.0.1:8080
```

Environment variables: `DATA_DIR` (./data), `BIND` (127.0.0.1:8080), `ASSETS_DIR` (.),
`CONTACT` (put in the User-Agent so data providers can reach you), `SST_DAYS` (90),
`PRECIP_DAYS` (120), `SENAMHI_INBOX` (import a relayed SENAMHI export instead of
fetching, see below), `SCHEDULER=off` (serve without fetching), `RUST_LOG`.

Tip: if the repo lives in Dropbox, set `CARGO_TARGET_DIR` outside it so build
artefacts are not synced.

## Deploy on the VPS (Caddy on host)

```sh
git clone … /opt/elnino && cd /opt/elnino
cp .env.example .env                     # set CONTACT and, if 8080 is taken, ELNINO_PORT
docker compose build                     # build on the VPS: native x86_64, no emulation
docker compose run --rm elnino backfill  # one-off: history + 90 SST frames
docker compose up -d
curl -s 127.0.0.1:${ELNINO_PORT:-8080}/healthz   # ok
```

Add `deploy/Caddyfile.example` to `/etc/caddy/Caddyfile` (change the domain, and the
port if you set `ELNINO_PORT`) and `sudo systemctl reload caddy`. Caddy handles TLS and
compression; the container is published only on `127.0.0.1:${ELNINO_PORT:-8080}`.

Backups: `deploy/backup.sh` (nightly cron) copies a consistent SQLite snapshot out of
the container. The SST grids (`/data/grids`) can be re-downloaded with `backfill`.

Updating: `git pull && docker compose build && docker compose up -d`.

### SENAMHI relay (home box)

SENAMHI drops connections from some server networks, including our VPS. A box on a
network SENAMHI accepts (here a Linux mini PC at home) runs the fetch once a day,
exports the stations and daily series to one JSON file, and copies it into an inbox on
the VPS with rsync over SSH. With `SENAMHI_INBOX` set, the VPS imports that file hourly
instead of fetching. The import validates every station, series id, date and value
before writing anything. If the file is more than 3 days old, the source shows an error
on the sources page.

On the VPS:

```sh
mkdir -p /opt/el-nino/inbox              # before compose creates it as root
echo 'SENAMHI_INBOX=/inbox/senamhi.json' >> .env
docker compose up -d
```

The relay copies the file with rsync over SSH, by default with your usual SSH key
into `INBOX=/opt/el-nino/inbox`. The key must work without a passphrase prompt, since
the timer runs with no terminal or agent. Optionally, give the relay a dedicated key
(`SSH_KEY`) restricted to writing files into the inbox (no shell, no forwarding), and
set `INBOX=` empty, because rrsync roots paths at the inbox. `rrsync` ships with
rsync: it is in `/usr/bin` on recent Debian/Ubuntu, and older releases keep it in
`/usr/share/doc/rsync/scripts/` to copy into place.

```
command="rrsync -wo /opt/el-nino/inbox",restrict ssh-ed25519 AAAA… senamhi-relay
```

On the relay box (needs Rust and rsync; files in `deploy/senamhi-relay/`):

```sh
cargo build --release && install -Dm755 target/release/elnino ~/.local/bin/elnino
install -Dm755 deploy/senamhi-relay/senamhi-relay.sh ~/.local/bin/senamhi-relay.sh
install -Dm644 deploy/senamhi-relay/senamhi-relay.env.example ~/.config/elnino-relay.env  # edit it
install -Dm644 -t ~/.config/systemd/user deploy/senamhi-relay/senamhi-relay.{service,timer}
ssh -o BatchMode=yes moc0@your-vps true          # must succeed without a prompt
systemctl --user daemon-reload
systemctl --user start senamhi-relay.service     # first run (~40 min); then check:
journalctl --user -u senamhi-relay -e
systemctl --user enable --now senamhi-relay.timer   # daily at 09:30 Lima time
sudo loginctl enable-linger "$USER"                 # run the timer without a login
```

Updating the relay: `git pull`, rebuild, and reinstall the binary.

## Layout

```
src/
  sources/   one module per source: fetch + pure parse functions (unit-tested)
  grid.rs    1° grids: ESRI ASCII parsing, binary storage, area means, XYZ PNG tiles
  web/       routes, page view-models (views.rs), JSON API (api.rs)
  scheduler.rs, fetch.rs, db.rs, config.rs
migrations/  SQLite schema
templates/   pages (minijinja)          i18n/  es.toml, en.toml
static/      css, js (one module per page), vendor libs, Natural Earth countries,
             simplified typical-impact layers (tools/make_impact_layers.py),
             Peru department mask (tools/make_peru_mask.py),
             learn/world.json land outline for the Learn page (tools/make_learn_world.py)
templates/learn/  lesson content for the Learn page, one file per language
```

## Notes and caveats

- OISST anomalies use a 1971–2000 base, so they run warmer than CPC's 1991–2020
  indices; the daily Niño 3.4 / 1+2 derived from them are labelled unofficial.
- Precipitation comes from CHIRPS v3.0 preliminary (UCSB CHC), a satellite estimate,
  not official SENAMHI data: daily values are 5-day pentad totals disaggregated with
  IMERG timing and are published ~2 days after each pentad ends (2–7 day lag). Department
  means use a mask built from Natural Earth 10m admin-1 boundaries (`tools/make_peru_mask.py`,
  committed as `static/geo/peru_mask.bin`); tiny departments (e.g. Callao) cover very few
  grid cells. Backfilling history from the CHIRPS v3.0 final archive is planned for later.
- SENAMHI station rainfall has no open API, and the station data tables are behind a
  CAPTCHA, which we never touch. Once a day we read the chart that the station map embeds
  for each station (`map_red_graf.php`, one request every 2 s, ~30 min per run). Real-time
  conventional stations chart 30 days of daily totals; automatic stations chart the last
  48 hours, hourly, which we keep for 10 days and add up into SENAMHI's day (07:00 to
  07:00 local time, labelled with the first date), counting only days with all 24 hours.
  Automatic stations within 3.5 km of a conventional one are skipped so a site is not
  counted twice. Values are provisional and unvalidated. Stations (table `station`) are
  placed in departments with the CHIRPS mask (snapping up to 0.2° for coastal stations).
  Department values are the mean of the stations reporting that day; the Peru table also
  names the wettest single station, since a mean hides local downpours. The SENAMHI
  columns end at the latest gauge day, usually a few days after the CHIRPS columns (each
  date is in the header). History builds up from the first run, which covers only the
  last 30 days.
- ENFEN publishes recent comunicados only as PDFs; their alert state is taken from the
  latest comunicado that states it in its title, and the UI says which one.
- The typical-impact map is a deliberately coarse sketch (ellipses), labelled as such.
- GDACS events are shown as "consistent with typical El Niño impacts", never as
  attributed to El Niño.
- Phase 2 candidates: NASA FIRMS fires, NOAA Coral Reef Watch, FEWS NET,
  SENAMHI avisos, ReliefWeb, TAO/TRITON subsurface, Peru anchoveta seasons.

## License

Code: [MIT](LICENSE). Bundled third-party files keep their own licenses:
Apache ECharts 5.6.0 (`static/vendor/echarts.min.js`, Apache-2.0),
MapLibre GL JS 4.7.1 (`static/vendor/maplibre-gl.*`, BSD-3-Clause),
Barlow, Barlow Semi Condensed and JetBrains Mono fonts (`static/vendor/fonts`, SIL OFL 1.1) and
Natural Earth country boundaries (`static/geo/countries.geojson`, `static/learn/world.json`, public domain).
Data shown by the site belongs to its publishers (NOAA, IGP, ENFEN, GDACS); see the
Sources page for attribution. Files in `tests/fixtures` are samples of those public
datasets, kept only for parser tests.
