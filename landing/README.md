# Dainiki landing page

Landing page and waitlist form for **Dainiki** (the GPUI journaling app by SMBL),
built with [Topcoat](https://github.com/tokio-rs/topcoat), Rust's full-stack,
server-rendered web framework from the Tokio team. Waitlist signups are stored
in a Cloudflare D1 database named `smbl-wishlist`.

## Layout

```
landing/
├── build.rs               # runs the Tailwind CLI in a Cargo build script
├── d1/
│   └── schema.sql         # schema for the `smbl-wishlist` D1 database
├── src/
│   ├── main.rs            # router: layout + pages + shared app state
│   ├── landing.rs         # root layout, home page, form-status types
│   ├── waitlist.rs        # waitlist form handler + D1 REST client
│   ├── styles/app.css     # Tailwind input: design tokens + base resets
│   └── landing/           # header, hero, showcase, manifesto, roadmap, footer
```

## Running it

```sh
cd landing
topcoat dev            # builds, bundles assets, serves http://127.0.0.1:3000
```

`topcoat dev` rebuilds and live-reloads on file changes. For a plain build:

```sh
cargo build --release
topcoat asset bundle --release   # bundles Tailwind CSS + assets next to the binary
./target/release/smbl-dainiki-landing
```

Override the bind address with `HOST` / `PORT`.

## Wishlist → Cloudflare D1

The waitlist has two storage paths, depending on how the app runs:

| Where | How it reaches D1 |
| --- | --- |
| Topcoat app (`topcoat dev` / the Rust binary) | Cloudflare REST API + an API token |
| Cloudflare Worker (`deploy/`) | Native D1 binding (`env.WAITLIST`) |

### 1. Create the database

```sh
curl -X POST https://api.cloudflare.com/client/v4/accounts/$CF_ACCOUNT_ID/d1/database \
  -H "Authorization: Bearer $CF_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name": "smbl-wishlist"}'
```

Copy the `uuid` from the response. That is the database id.

### 2. Create the table

```sh
curl -X POST https://api.cloudflare.com/client/v4/accounts/$CF_ACCOUNT_ID/d1/database/$CF_DATABASE_ID/query \
  -H "Authorization: Bearer $CF_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"sql": "CREATE TABLE IF NOT EXISTS waitlist (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, source TEXT NOT NULL DEFAULT '\''landing'\'', created_at TEXT NOT NULL DEFAULT (datetime('\''now'\'')));"}'
```

(The same statement lives in `d1/schema.sql`.)

### 3. Run the Topcoat app against it

```sh
export CLOUDFLARE_ACCOUNT_ID="..."        # Cloudflare account id
export CLOUDFLARE_D1_DATABASE_ID="..."    # UUID of the smbl-wishlist database
export CLOUDFLARE_API_TOKEN="..."         # API token with D1 Write
topcoat dev
```

The API token needs the **D1 Write** permission on the account. With credentials
missing the form still renders and returns a friendly error; nothing is
silently dropped without an error in the server log.

`CLOUDFLARE_API_BASE` can point the client at a mock server for tests:

```sh
node scripts/mock-d1.mjs &   # mock D1 REST API on 127.0.0.1:8787
CLOUDFLARE_API_BASE=http://127.0.0.1:8787/client/v4 \
CLOUDFLARE_ACCOUNT_ID=dev CLOUDFLARE_D1_DATABASE_ID=dev CLOUDFLARE_API_TOKEN=dev \
./target/debug/smbl-dainiki-landing
```

The mock records inserts, returns `changes: 0` for duplicates, and exposes
`GET /__seen` so you can exercise the joined / duplicate / invalid / error
states of the form without touching a real D1 database.

## Deploy on Cloudflare Workers

Topcoat's tokio server can't run inside workerd, so the page ships as
**Workers Static Assets**, exported from the Topcoat app, plus a slim Worker
(`deploy/src/index.js`) that owns the two dynamic bits: the `POST /waitlist`
handler (D1 binding, no API token needed) and injecting the confirmation line
into the page for `GET /?joined=<code>`.

Everything under `deploy/` is versioned; `deploy/public/` is generated.

```sh
# 1. Export the rendered page + hashed CSS/images (from the debug or
#    release binary; --release builds must exist first)
node deploy/export.mjs

# 2. Deploy assets, Worker and the custom domain
cd deploy
npx wrangler deploy
```

`wrangler.jsonc` binds the `smbl-wishlist` database as `env.WAITLIST` and maps
the custom domain `dainiki.smbl.dev`. If a previous DNS record (for example a
tunnel CNAME) already owns that hostname, Cloudflare rejects the custom-domain
attach with code 100117 until the record is deleted.

`GET /` is served as a plain static asset (edge-cached); only the
`?joined=...` variant runs the Worker, which replaces the
`<!--WAITLIST_STATUS-->` placeholder with the status paragraph, so the
confirmation renders even with JavaScript disabled.

## Notes

- **Duplicates** are harmless: the insert uses `ON CONFLICT(email) DO NOTHING`
  and the UI confirms either way without revealing whether the email was new.
- A hidden honeypot field (`website`) catches naive bots; filled fields are
  accepted but never stored.
- Validation happens server-side in `waitlist::is_valid_email` (unit-tested).
- The showcase photos and logo are hotlinked from the original prototype;
  move them into `src/` and reference them with `asset!("./...")` before going
  to production.
