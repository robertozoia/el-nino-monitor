-- Rain-gauge stations (SENAMHI), refreshed from the station list on every run.
-- id is the station's daily series id in observation, so the two join directly.
CREATE TABLE station (
    id        TEXT PRIMARY KEY,    -- senamhi_st_<code>
    source_id TEXT NOT NULL,
    code      TEXT NOT NULL,
    name      TEXT NOT NULL,
    kind      TEXT NOT NULL,       -- conventional | automatic
    dept      TEXT,                -- department code (CHIRPS mask)
    lat       REAL NOT NULL,
    lon       REAL NOT NULL,
    updated   TEXT NOT NULL
);
CREATE INDEX station_dept ON station(dept);
