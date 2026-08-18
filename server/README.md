# ironpress-server

A small HTTP server that exposes the [ironpress](../) HTML-to-PDF engine over a
**Gotenberg-compatible** multipart form API. It is a separate workspace crate so
the core `ironpress` library stays free of web-server dependencies.

## Running

```sh
cargo run -p ironpress-server --release
# ironpress-server 0.1.0 listening on http://0.0.0.0:3000 (sanitize=true, max_body=67108864 bytes)
```

### Startup configuration (operator-controlled, environment variables)

These fix the process's behaviour and **cannot** be overridden by a request.
Anything that affects the security posture lives here by design.

| Variable                  | Default      | Meaning                                              |
| ------------------------- | ------------ | ---------------------------------------------------- |
| `IRONPRESS_PORT`          | `3000`       | TCP port to listen on.                               |
| `IRONPRESS_SANITIZE`      | `true`       | Sanitize HTML before conversion. Disable only for fully trusted input. |
| `IRONPRESS_MAX_BODY_BYTES`| `67108864`   | Maximum request body size (64 MiB).                  |
| `IRONPRESS_REMOTE_ENABLED`| `false`      | Allow fetching remote document resources. Off = every remote reference is blocked before any connection. |

#### Remote resources (SSRF surface)

By default (`IRONPRESS_REMOTE_ENABLED=false`) the server fetches **nothing** from
the network: `<img src="https://…">` and remote `@import` are rejected before any
DNS/TCP, and only files you upload (staged in a private per-request temp
directory) plus `data:` URIs load. The document still converts — blocked
resources are simply absent.

Set `IRONPRESS_REMOTE_ENABLED=true` to allow outbound fetching. The policy is
then built **only** from these startup variables — never from a request — with
SSRF-hardened defaults (all non-public address ranges, including loopback,
private, link-local and cloud-metadata IPs, are denied):

| Variable                          | Default | Meaning                                                        |
| --------------------------------- | ------- | -------------------------------------------------------------- |
| `IRONPRESS_REMOTE_ALLOW_HOSTS`    | —       | Comma-separated host allow-list. `example.com` = exact; `.example.com` = subdomains. An allow entry bypasses the IP-class checks (DNS pinning still applies). |
| `IRONPRESS_REMOTE_DENY_HOSTS`     | —       | Comma-separated host deny-list (wins over allow).              |
| `IRONPRESS_REMOTE_DENY_PRIVATE_IPS`| `true` | Reject non-public target addresses.                            |
| `IRONPRESS_REMOTE_DENY_PUBLIC_IPS`| `false` | Reject public target addresses.                                |
| `IRONPRESS_REMOTE_MAX_REDIRECTS`  | `8`     | Max redirects (each hop re-checked).                           |
| `IRONPRESS_REMOTE_MAX_BODY_BYTES` | (lib default) | Max remote response size.                                |

A malformed host pattern fails startup. For a hard no-egress guarantee, also cut
egress at the network layer (`docker run --network none`, a k8s `NetworkPolicy`,
or no route out of the pod) — that is independent of this flag.

> **No sensitive knobs are request-controllable.** Sanitization and the entire
> remote/allow-list/IP policy are startup-only. Per-request form fields can only
> change cosmetic rendering options.

## Routes

### `POST /convert/html`

`multipart/form-data`. Files are identified by their `filename`:

| File          | Required | Purpose                                             |
| ------------- | -------- | --------------------------------------------------- |
| `index.html`  | yes      | The document to convert.                             |
| `header.html` | no       | Running header (see header/footer note below).      |
| `footer.html` | no       | Running footer.                                      |
| *other files* | no       | Assets referenced by **relative** URL from the document (CSS via `@import`, images, fonts). |

Response: `application/pdf`. The output filename comes from the optional
`Gotenberg-Output-Filename` request header (default `output.pdf`).

#### Options (form fields)

Geometry fields use **inches** (Gotenberg convention) and are converted to points.

| Field                 | Unit  | Default | Notes                                          |
| --------------------- | ----- | ------- | ---------------------------------------------- |
| `paperWidth`          | in    | 8.27 (A4) | Page width.                                   |
| `paperHeight`         | in    | 11.69 (A4)| Page height.                                  |
| `marginTop/Bottom/Left/Right` | in | 1     | Per-side margins.                             |
| `landscape`           | bool  | `false` | Swaps width/height.                            |
| `preferCssPageSize`   | bool  | —       | Accepted; ironpress already honors `@page`.    |
| `printBackground`     | bool  | —       | Accepted; ironpress always paints backgrounds. |
| `compress`            | bool  | `true`  | FlateDecode content streams.                   |
| `imageDpi`            | DPI   | `300`   | Target source-image resolution.                |
| `autoResizeImages`    | bool  | `true`  | Downscale oversized images (Lanczos3, shrink-only). |
| `jpegQuality`         | 0–100 | `95`    | Re-encode quality.                             |
| `filterDpi`           | DPI   | `300`   | Blur/filter raster resolution.                 |
| `maskDpi`             | DPI   | `300`   | CSS-mask raster resolution.                    |
| `backgroundRasterDpi` | DPI   | `192`   | Flattened-background resolution.               |
| `occlusionCull`       | bool  | `false` | Skip images fully covered by later opaque boxes. |

CSS `@page { size; margin }` in the document still overrides page geometry, as in
the library. Browser-only Gotenberg fields (`scale`, `waitDelay`,
`emulatedMediaType`, `nativePageRanges`, JavaScript options, …) are **accepted
and ignored** — ironpress has no browser.

#### Header / footer

ironpress running headers/footers are a single line of **text** with `{page}` /
`{pages}` placeholders, not full HTML. `header.html` / `footer.html` are
translated best-effort:

- `<span class="pageNumber"></span>` → `{page}`
- `<span class="totalPages"></span>` → `{pages}`
- all other markup is reduced to its text content (styling is lost).

### `GET /health`

`{"status":"up"}` — liveness probe.

### `GET /version`

`{"ironpress":"<lib version>","server":"<server version>"}`.

## Example

```sh
curl -s -o out.pdf http://localhost:3000/convert/html \
  -F "index.html=@index.html;filename=index.html" \
  -F "header.html=@header.html;filename=header.html" \
  -F "footer.html=@footer.html;filename=footer.html" \
  -F "paperWidth=8.5" -F "paperHeight=11" -F "marginTop=0.5" \
  -H "Gotenberg-Output-Filename: invoice"
```

## Docker

A single image is built from the repository root (it builds the `ironpress`
library and this server). One artifact, configured per environment via env vars.

```sh
# Build (from the repo root, where the Dockerfile lives)
docker build -t ironpress-server .

# Run — remote fetching off, no network egress
docker run --rm -p 3000:3000 ironpress-server

# Run — allow remote assets from one CDN only
docker run --rm -p 3000:3000 \
  -e IRONPRESS_REMOTE_ENABLED=true \
  -e IRONPRESS_REMOTE_ALLOW_HOSTS=cdn.example.com \
  ironpress-server

# Hardened: block egress at the network layer regardless of app config
docker run --rm -p 3000:3000 --network none ironpress-server
```

The image runs as a non-root user and ships a self-contained `HEALTHCHECK`
(`ironpress-server --health`, a TCP liveness probe — no curl/wget needed).
