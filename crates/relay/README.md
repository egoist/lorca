# lorca-relay

The relay stores public keys and ciphertext and forwards blobs between an identity's Devices. The [Relay doc](../../web/content/docs/relay.mdx) covers running it and [ARCHITECTURE.md](../../ARCHITECTURE.md#relay) the protocol and storage.

Every flag has an environment variable, listed by `lorca-relay --help`.

## Deploy on Railway

1. Create a service from this repository and leave **Root Directory** empty. [`railway.toml`](../../railway.toml) at the repo root builds this [`Dockerfile`](Dockerfile) with the whole Cargo workspace as context, redeploys only when `crates/relay/`, `Cargo.toml`, or `Cargo.lock` change, and healthchecks `/v1/health`.
2. Set the [variables every deploy needs](#variables-every-deploy-needs).
3. Pick storage: [SQLite on a volume](#sqlite-on-a-volume) or [Postgres and a bucket](#postgres-and-a-bucket).
4. Generate a domain under **Settings › Networking**.
5. Point your first Device at `https://<domain>`: **Settings › Advanced › Relay URL** in the app, or `LORCA_RELAY_URL` for the CLI. Devices you pair afterwards learn the URL from the pairing code.

Once it is up, `https://<domain>` reads "Lorca Relay is running...", `curl https://<domain>/v1/health` answers `{"ok":true,"service":"lorca-relay"}`, and the relay's first log line names its address, database, file store, and push services.

### Variables every deploy needs

| Variable | Value |
| --- | --- |
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
| `LORCA_RELAY_APNS_KEY` | `/data/apns.p8`: the APNs key from developer.apple.com › Keys (Apple Push Notifications service). |
| `LORCA_RELAY_APNS_KEY_ID` | The key's 10-character id, also in the `.p8` file name. |
| `LORCA_RELAY_APNS_TEAM_ID` | The Apple team id. |
| `LORCA_RELAY_APNS_TOPIC` | The phone app's bundle id; `app.lorca` by default. |
| `LORCA_RELAY_FCM_SERVICE_ACCOUNT` | `/data/fcm.json`: a key from Firebase console › Project settings › Service accounts. |

The relay reads both keys from files, so they live on the volume. Copy them in with [`railway ssh`](https://docs.railway.com/cli/ssh) from a directory linked to the project (register an SSH key once with `railway ssh keys add` or `railway ssh keys github`), then set the variables:

```bash
railway ssh --service <relay service> -- sh -c 'cat > /data/apns.p8' < AuthKey_XXXXXXXXXX.p8
```

```bash
railway ssh --service <relay service> -- sh -c 'cat > /data/fcm.json' < service-account.json
```

Copy the files first: the relay does not start while a key path is missing. Its first log line then shows `push=apns`, `push=fcm`, or `push=apns+fcm`. The Postgres setup has no volume; adding one for the keys brings back deploys that stop the old relay first.

### Optional

| Variable | Default | What it does |
| --- | --- | --- |
| `LORCA_RELAY_QUOTA_BYTES` | `0` | Stored ciphertext allowed per identity, in bytes. `0` means no limit. |
| `LORCA_RELAY_IP_PER_MINUTE` | `60` | Requests per minute one IP may make to registration, auth, and the pairing mailbox. |
| `LORCA_RELAY_IDENTITY_PER_SECOND` | `50` | Requests per second one identity may make across its machines, with a burst of ten times that. |
| `RUST_LOG` | `info` | `info,lorca_relay=debug` also logs each rate-limited request with the address it counted against. |

## Run the image elsewhere

The image builds from the repo root and listens on 8787 when `PORT` is unset. The variables above go in with `-e`.

```bash
docker build -f crates/relay/Dockerfile -t lorca-relay .
```

```bash
docker run -p 8787:8787 -v lorca-relay:/data lorca-relay
```
