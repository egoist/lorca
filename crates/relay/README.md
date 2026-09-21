# lorca-relay

The relay stores public keys and ciphertext and forwards blobs between an identity's Devices. The [Relay doc](../../web/content/docs/relay.mdx) covers running it and [ARCHITECTURE.md](../../ARCHITECTURE.md#relay) the protocol and storage.

Every flag has an environment variable, listed by `lorca-relay --help`.

## Deploy on Railway

1. Create a service from this repository and leave **Root Directory** empty: the [`Dockerfile`](Dockerfile) builds with the whole Cargo workspace as context. In the service's **Settings**, set **Healthcheck Path** to `/v1/health`, and **Watch Paths** to `/crates/relay/**`, `/Cargo.toml`, and `/Cargo.lock` so that pushes elsewhere in the repo leave the relay running.
2. Set the [variables every deploy needs](#variables-every-deploy-needs).
3. Pick storage: [SQLite on a volume](#sqlite-on-a-volume) or [Postgres and a bucket](#postgres-and-a-bucket).
4. Generate a domain under **Settings › Networking**.
5. Point your first Device at `https://<domain>`: **Settings › Advanced › Relay URL** in the app, or `LORCA_RELAY_URL` for the CLI. Devices you pair afterwards learn the URL from the pairing code.

Once it is up, `https://<domain>` reads "Lorca Relay is running...", `curl https://<domain>/v1/health` answers `{"ok":true,"service":"lorca-relay"}`, and the relay's first log line names its address, database, file store, and push services.

### Variables every deploy needs

| Variable | Value |
| --- | --- |
| `RAILWAY_DOCKERFILE_PATH` | `crates/relay/Dockerfile`. Railway builds the service from this file. |
| `LORCA_RELAY_SECRET` | The output of `openssl rand -hex 32`. It signs bearer tokens; unset, it changes on every boot and invalidates every Device's token. |
| `LORCA_RELAY_TRUST_PROXY` | `true`. The relay takes the client address from the `X-Forwarded-For` header that Railway's edge sets; otherwise every request comes from the proxy and all clients share one rate limit. |

Leave `PORT` and `LORCA_RELAY_BIND` unset. Railway sets `PORT`, the image listens on `[::]:$PORT`, and the healthcheck calls that port. `LORCA_RELAY_BIND` takes precedence over `PORT`.

### SQLite on a volume

Add a volume mounted at `/data`; no variable is needed. The image runs in `/data`, so the relay keeps `/data/lorca-relay.db` and `/data/lorca-relay.files`.

Railway runs one deployment at a time on a volume: a deploy stops the old relay before the new one starts, and Devices reconnect a few seconds later.

### Postgres and a bucket

With Postgres, Railway starts the new deployment before it stops the old one, and the service can run more than one replica. Add Railway's Postgres to the project and give the relay no volume.

| Variable | Value |
| --- | --- |
| `LORCA_RELAY_DB` | `${{Postgres.DATABASE_URL}}?sslmode=disable` |
| `LORCA_RELAY_S3_BUCKET` | The bucket's name. |
| `LORCA_RELAY_S3_ENDPOINT` | `https://<account id>.r2.cloudflarestorage.com`, `https://s3.us-east-1.amazonaws.com`, … |
| `LORCA_RELAY_S3_ACCESS_KEY` | Falls back to `AWS_ACCESS_KEY_ID`. |
| `LORCA_RELAY_S3_SECRET_KEY` | Falls back to `AWS_SECRET_ACCESS_KEY`. |
| `LORCA_RELAY_S3_REGION` | The SigV4 region; `auto` (R2) by default. |
| `LORCA_RELAY_S3_PREFIX` | A key prefix inside the bucket; empty by default. |

`Postgres` in the reference is the database service's name. `DATABASE_URL` reaches the database over the private network (`postgres.railway.internal`). Its certificate is self-signed and the relay checks certificates against the web's roots, so without `sslmode=disable` the relay exits at startup with `invalid peer certificate: UnknownIssuer`.

Attachments go to the bucket because a deploy replaces the container's disk. The relay addresses objects path-style (`<endpoint>/<bucket>/<key>`), which R2, S3, and MinIO accept; a Railway Bucket accepts it when its **Credentials** tab says path-style.

### Push notifications

| Variable | Value |
| --- | --- |
| `LORCA_RELAY_APNS_KEY` | The text of the APNs key from developer.apple.com › Keys (Apple Push Notifications service): paste the `.p8` file, `-----BEGIN PRIVATE KEY-----` line included. |
| `LORCA_RELAY_APNS_KEY_ID` | The key's 10-character id, also in the `.p8` file name. |
| `LORCA_RELAY_APNS_TEAM_ID` | The Apple team id. |
| `LORCA_RELAY_APNS_TOPIC` | The phone app's bundle id; `app.lorca` by default. |
| `LORCA_RELAY_FCM_SERVICE_ACCOUNT` | The JSON of a key from Firebase console › Project settings › Service accounts. |

Both keys go in as text, so the service needs no volume and the Postgres setup keeps its deploys without a gap. A key with `\n` in place of its line breaks works too. Either variable also takes the path of a file (`/data/apns.p8`), for a relay that has a volume anyway. The relay's first log line shows `push=apns`, `push=fcm`, or `push=apns+fcm`.

### Optional

| Variable | Default | What it does |
| --- | --- | --- |
| `LORCA_RELAY_QUOTA_BYTES` | `5368709120` | Stored ciphertext allowed per identity, in bytes (5 GiB). `0` means no limit. |
| `LORCA_RELAY_CONCURRENT_UPLOADS` | `3` | Uploads over 1 MiB handled at once. Each takes some 80 MB while it is decoded and sent to the bucket; lower it on a small container. `0` means no limit. |
| `LORCA_RELAY_IP_PER_MINUTE` | `60` | Requests per minute one IP may make to registration, auth, and the pairing mailbox. |
| `LORCA_RELAY_IDENTITY_PER_SECOND` | `50` | Requests per second one identity may make across its machines, with a burst of ten times that. |
| `LORCA_RELAY_MIN_PROTOCOL` | `0` | Clients that speak an older relay protocol get `426`, and their apps ask for an update. `/v1/health` shows the protocol this relay speaks. Raise it only once the Lorca versions you care about have shipped the newer one. |
| `LORCA_RELAY_INACTIVE_DAYS` | `365` | An identity with no machine seen, no blob written, and no socket open for this many days is deleted with its attachments. Its Devices keep what they hold and register again if they come back. `0` keeps every identity. |
| `LORCA_RELAY_METRICS_TOKEN` | unset | Serves `GET /metrics` in Prometheus' text format to a scraper that sends this as a bearer token: `openssl rand -hex 32`. Unset, the route answers `404`. With more than one replica a scrape reaches one of them; request, push, and sweep counters carry its `instance`, and the totals are the same from each. |
| `RUST_LOG` | `info` | `info,lorca_relay=debug` also logs each rate-limited request with the address it counted against. |

## Run the image elsewhere

The image builds from the repo root and listens on 8787 when `PORT` is unset. The variables above go in with `-e`.

```bash
docker build -f crates/relay/Dockerfile -t lorca-relay .
```

```bash
docker run -p 8787:8787 -v lorca-relay:/data lorca-relay
```
