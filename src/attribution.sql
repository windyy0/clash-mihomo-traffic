-- Sparse, transactional redirects. Original process facts are never rewritten.
CREATE TABLE IF NOT EXISTS owner_metadata(key TEXT PRIMARY KEY,label TEXT NOT NULL,icon TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS app_attribution_minutes(time INTEGER,app INTEGER,owner INTEGER,up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,app,owner));
CREATE INDEX IF NOT EXISTS attribution_owner ON app_attribution_minutes(owner,time);
CREATE TABLE IF NOT EXISTS attribution_details(time INTEGER,app INTEGER,owner INTEGER,domain INTEGER,route INTEGER,source INTEGER,up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,app,owner,domain,route,source));
CREATE INDEX IF NOT EXISTS attribution_detail_owner ON attribution_details(owner,time);
CREATE TABLE IF NOT EXISTS attribution_daily(time INTEGER,kind TEXT,dim INTEGER,app INTEGER,owner INTEGER,up INTEGER NOT NULL,down INTEGER NOT NULL,PRIMARY KEY(time,kind,dim,app,owner));

CREATE TABLE IF NOT EXISTS package_app_ids(app INTEGER PRIMARY KEY,owner INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS package_owner_apps ON package_app_ids(owner);

CREATE VIEW IF NOT EXISTS application_minutes AS
SELECT f.time,COALESCE(p.owner,f.app) AS app,SUM(f.up) AS up,SUM(f.down) AS down FROM (
    SELECT time,app,up,down FROM app_minutes
    UNION ALL SELECT time,app,-up,-down FROM app_attribution_minutes
    UNION ALL SELECT time,owner,up,down FROM app_attribution_minutes
) f LEFT JOIN package_app_ids p ON p.app=f.app GROUP BY f.time,COALESCE(p.owner,f.app) HAVING SUM(up)>0 OR SUM(down)>0;

CREATE VIEW IF NOT EXISTS application_process_minutes AS
SELECT f.time,COALESCE(p.owner,f.app) AS app,f.process,SUM(f.up) AS up,SUM(f.down) AS down FROM (
    SELECT time,app,app AS process,up,down FROM app_minutes
    UNION ALL SELECT time,app,app,-up,-down FROM app_attribution_minutes
    UNION ALL SELECT time,owner,app,up,down FROM app_attribution_minutes
) f LEFT JOIN package_app_ids p ON p.app=f.app GROUP BY f.time,COALESCE(p.owner,f.app),process HAVING SUM(up)>0 OR SUM(down)>0;

CREATE VIEW IF NOT EXISTS application_details AS
SELECT f.time,COALESCE(p.owner,f.app) AS app,f.domain,f.route,f.source,SUM(f.up) AS up,SUM(f.down) AS down FROM (
    SELECT time,app,domain,route,source,up,down FROM details
    UNION ALL SELECT time,app,domain,route,source,-up,-down FROM attribution_details
    UNION ALL SELECT time,owner,domain,route,source,up,down FROM attribution_details
) f LEFT JOIN package_app_ids p ON p.app=f.app GROUP BY f.time,COALESCE(p.owner,f.app),domain,route,source HAVING SUM(up)>0 OR SUM(down)>0;

CREATE VIEW IF NOT EXISTS application_daily AS
SELECT f.time,f.kind,f.dim,COALESCE(p.owner,f.app) AS app,SUM(f.up) AS up,SUM(f.down) AS down FROM (
    SELECT time,kind,dim,app,up,down FROM daily
    UNION ALL SELECT time,kind,dim,app,-up,-down FROM attribution_daily
    UNION ALL SELECT time,kind,dim,owner,up,down FROM attribution_daily
) f LEFT JOIN package_app_ids p ON p.app=f.app GROUP BY f.time,f.kind,f.dim,COALESCE(p.owner,f.app) HAVING SUM(up)>0 OR SUM(down)>0;
