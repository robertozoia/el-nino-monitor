-- Fetch status per source (one row per entry in sources::REGISTRY).
CREATE TABLE source_status (
    id            TEXT PRIMARY KEY,
    last_attempt  TEXT,
    last_success  TEXT,
    last_change   TEXT,
    status        TEXT,            -- ok | unchanged | error
    message       TEXT,
    fail_count    INTEGER NOT NULL DEFAULT 0
);

-- Conditional-GET bookkeeping per URL.
CREATE TABLE http_cache (
    url           TEXT PRIMARY KEY,
    etag          TEXT,
    last_modified TEXT,
    content_hash  TEXT,
    fetched_at    TEXT
);

CREATE TABLE fetch_log (
    id          INTEGER PRIMARY KEY,
    source_id   TEXT NOT NULL,
    at          TEXT NOT NULL,
    outcome     TEXT NOT NULL,     -- changed | unchanged | error
    ms          INTEGER,
    message     TEXT
);
CREATE INDEX fetch_log_source ON fetch_log(source_id, at);

-- Scalar time series (indices). t is ISO date (YYYY-MM-DD; monthly = first of month).
CREATE TABLE series (
    id        TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    unit      TEXT NOT NULL,
    freq      TEXT NOT NULL        -- weekly | monthly | daily
);

CREATE TABLE observation (
    series_id TEXT NOT NULL,
    t         TEXT NOT NULL,
    value     REAL NOT NULL,
    PRIMARY KEY (series_id, t)
) WITHOUT ROWID;

-- Forecast tables, one row per (product, issue, season). data is a JSON array/object.
CREATE TABLE forecast (
    product TEXT NOT NULL,         -- cpc_roni_outlook | cpc_probabilities | cpc_strength
    issued  TEXT NOT NULL,         -- YYYY-MM
    season  TEXT NOT NULL,         -- SON, OND, ...
    ord     INTEGER NOT NULL,
    data    TEXT NOT NULL,
    PRIMARY KEY (product, issued, season)
);

-- Official statements: ENFEN comunicados, CPC discussions.
CREATE TABLE document (
    id        TEXT PRIMARY KEY,    -- enfen:18-2026, cpc_disc:2026-10-08
    source_id TEXT NOT NULL,
    number    TEXT,
    issued    TEXT NOT NULL,       -- YYYY-MM-DD
    title     TEXT NOT NULL,
    status    TEXT,                -- alert status as published
    summary   TEXT,
    url       TEXT,
    pdf_url   TEXT,
    extra     TEXT                 -- JSON
);
CREATE INDEX document_source ON document(source_id, issued);

-- Hazard events (GDACS).
CREATE TABLE event (
    id         TEXT PRIMARY KEY,   -- gdacs:TC:1001338
    source_id  TEXT NOT NULL,
    type       TEXT NOT NULL,      -- FL | DR | TC | WF
    name       TEXT NOT NULL,
    alert      TEXT NOT NULL,      -- Green | Orange | Red
    countries  TEXT,
    iso3       TEXT,
    lon        REAL,
    lat        REAL,
    region     TEXT NOT NULL,
    from_date  TEXT NOT NULL,
    to_date    TEXT,
    modified   TEXT,
    severity   TEXT,
    url        TEXT,
    is_current INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX event_from ON event(from_date);

-- Daily gridded frames stored on disk under DATA_DIR/grids.
CREATE TABLE grid_frame (
    dataset TEXT NOT NULL,
    date    TEXT NOT NULL,
    created TEXT NOT NULL,
    PRIMARY KEY (dataset, date)
);

-- Curated links to low-frequency publications (WMO, BoM, JMA...).
CREATE TABLE link (
    id     INTEGER PRIMARY KEY,
    org    TEXT NOT NULL,
    title  TEXT NOT NULL,
    url    TEXT NOT NULL UNIQUE,
    issued TEXT,
    lang   TEXT
);

INSERT INTO link (org, title, url, lang) VALUES
  ('WMO',     'El Niño / La Niña Updates',                  'https://wmo.int/topics/el-nino-la-nina', 'en'),
  ('NOAA',    'ENSO Blog (Climate.gov)',                    'https://www.climate.gov/news-features/department/enso-blog', 'en'),
  ('BoM',     'Climate Driver Update',                      'http://www.bom.gov.au/climate/enso/', 'en'),
  ('JMA',     'El Niño Outlook',                            'https://ds.data.jma.go.jp/tcc/tcc/products/elnino/outlook.html', 'en'),
  ('IRI',     'ENSO Forecast (model plume)',                'https://iri.columbia.edu/our-expertise/climate/forecasts/enso/current/', 'en'),
  ('C3S',     'Copernicus seasonal forecasts',              'https://climate.copernicus.eu/seasonal-forecasts', 'en'),
  ('CIIFEN',  'Centro Internacional para la Investigación del Fenómeno de El Niño', 'https://ciifen.org/', 'es'),
  ('SENAMHI', 'Avisos meteorológicos — Perú',               'https://www.senamhi.gob.pe/?p=aviso-meteorologico', 'es'),
  ('FEWS NET','Food security outlooks',                     'https://fews.net/', 'en');
