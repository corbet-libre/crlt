# Dependency maintenance

Release review on 2026-09-30 used the [crates.io release list](https://crates.io/crates/libsql/versions)
and the published manifests for [0.9.30](https://docs.rs/crate/libsql/0.9.30/source/Cargo.toml)
and [0.10.0-pre.4](https://docs.rs/crate/libsql/0.10.0-pre.4/source/Cargo.toml).
0.9.30 remains the latest stable; both versions require hyper 0.14 with HTTP/2
and hyper-rustls 0.25 for the built-in remote transport. A release bump cannot
resolve the old h2, rustls-webpki and rustls-pemfile advisory chain.

crlt enables only libsql's `core` feature. The upstream generic Hrana client
and `HttpSend` interface are private; injecting a current sender would require
a fork. Instead, crlt uses the published `libsql-hrana` protocol structures with
reqwest 0.13.5/rustls 0.23. The adapter implements only the [Hrana 3 HTTP pipeline](https://github.com/tursodatabase/libsql/blob/main/docs/HRANA_3_SPEC.md):
execute, get-autocommit and close, with one baton per exclusive pool connection.
Official types provide value encoding (including base64 blobs and full-width
integers); there is no fork, Cargo patch or custom cryptography. Deadpool still
owns pooling. The public crlt API, scoped SQL rewriting, schema audit and index
checks are unchanged. Abandoned remote leases schedule bounded Close requests,
which roll back the stream. Lost replies may require server expiry to release
an unknown/stale baton; writes are never retried automatically.

Pipeline responses are buffered up to a fixed 16 MiB wire-body limit, enforced
before JSON decoding even without Content-Length. The existing `max_rows` limit
still applies during row collection. Oversized replies fail with a database
error; large result sets should be paged. This deliberately small adapter does
not implement cursors, replication, WebSockets, or client-side SQL parsing.
Malformed replies and cancelled requests invalidate the stream; a later call
cannot silently open a replacement transaction.

The HTTPS client uses platform trust roots and verifies certificates and hostnames.
It rejects HTTP redirects and plaintext, including Hrana-provided base URLs,
and disables automatic retries and proxy discovery. The trusted database server
may route a stream to another HTTPS base URL (Turso's sticky routing); that server
is part of the credential trust boundary. HTTP/1 is sufficient for Hrana and
avoids adding HTTP/2 machinery. The User-Agent is `crlt`. Transport errors contain
neither URLs nor response bodies. No logging subscriber or request logs are added.

CI checks advisories, licenses, bans and sources across the complete dependency
graph, without target exclusions. Bans prevent the obsolete HTTP/TLS chain from
returning through feature unification. No TLS advisory exceptions are permitted.
The lockfile records the graph tested on CI; security upgrades must retain the
local and TLS sqld round trips. Downstream crates must update their crlt revision
and lockfile separately, and must not re-enable libsql's legacy transport features.

The committed CI resolution uses reqwest 0.13.5, hyper 1.11.1,
hyper-rustls 0.27.10, rustls 0.23.45, rustls-webpki 0.103.15 and
rustls-native-certs 0.8.4. Neither h2 nor rustls-pemfile is present. Local tests
and the authenticated TLS sqld suite validate that same locked graph.

There are currently no advisory exceptions. Any future non-TLS exception must
identify the advisory, dependency, exposure, reason and removal condition here
and in `deny.toml`; a passing audit is never a claim that an ignored issue is fixed.
