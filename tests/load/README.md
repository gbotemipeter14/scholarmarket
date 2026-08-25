# Load Testing — `GET /api/materials/search`

This directory contains load tests for the marketplace search endpoint, the
single most heavily used route in ScholarMarket. It satisfies issue #39.

## What is tested

| Tool     | File                              | Notes                                  |
| -------- | --------------------------------- | -------------------------------------- |
| **k6**   | `materials-search.k6.js`          | Primary. Constant 100 RPS + 120-VU spike, thresholds for p95<500ms and error rate <1%. |
| **Artillery** | `materials-search.yml`      | Alternative. Ramps to 150 arrival/sec with a variety of queries. |

Both scripts fire a *varied* set of queries (free-text `q`, `subject`,
`category`, `level`, `licenseType`, `sortBy`, pagination) so the cache and
MongoDB query planner are exercised realistically rather than hammering one
cached key.

## Prerequisites

1. A running MongoDB seeded with materials (`npm run dev` expects `MONGODB_URI`).
2. The app running: `npm run dev` (default `http://localhost:3000`).

## Running

```bash
# k6 (https://k6.io/docs/get-started/installation/)
export BASE_URL=http://localhost:3000
k6 run tests/load/materials-search.k6.js

# Artillery (npm install -g artillery)
BASE_URL=http://localhost:3000 artillery run tests/load/materials-search.yml
```

k6 prints throughput (`http_reqs`), latency (`http_req_duration` → avg/p95/p99)
and, via the `error_rate` custom metric, the failure ratio. Artillery prints a
`Latency` block (median/p95/p99) and a `Codes` block (HTTP status distribution
= error rate).

## Acceptance targets (from the issue)

- **Throughput:** ≥ 100 requests/second.
- **Latency:** average response time < 500 ms (we additionally assert p95 < 500 ms, p99 < 1000 ms).
- **Error rate:** < 1 % of responses non-2xx.

## Identified bottlenecks & mitigations

1. **Unindexed search/filter predicates.** `buildMarketplaceDiscoveryQuery`
   builds `$regex` / `$in` filters on `fileType`, `contentType`, `subject`,
   `category`, `level`, `creator`, `licenseType` and a `$text` search on `q`.
   With no supporting indexes a 100 RPS stream triggers collection scans.
   *Mitigation:* add compound indexes, e.g.
   `{ visibility: 1, status: 1, createdAt: -1 }` and a `text` index on the
   searchable fields; ensure `subject`/`category`/`level` are indexed.

2. **Cache-key cardinality.** The Redis key is the full (ordered) query string
   with a 300 s TTL. Highly varied free-text queries produce mostly cache
   *misses*, so every miss is a synchronous MongoDB hit. At 100 RPS this is the
   dominant latency source.
   *Mitigation:* keep a short-TTL cache only for the most common/empty queries,
   and/or pre-compute popular result pages; consider a stale-while-revalidate
   strategy to absorb spikes.

3. **Synchronous, per-request DB round-trips.** Each request opens a cursor and
   runs `countDocuments` + `find`. `countDocuments` is expensive on large
   collections. *Mitigation:* use a capped/scrolled estimate for `total`, or
   drop `total` under load and rely on keyset pagination.

4. **In-memory rate limiting.** `withApiHardening` keeps the rate-limit counter
   in process memory. Behind multiple app instances (horizontal scaling) the
   limit is per-instance, not global, and a single hot instance can still be
   overwhelmed. *Mitigation:* move the rate-limit counter to Redis
   (`REDIS_URL`) so it aggregates across instances.

5. **Single-instance Node throughput.** Even with indexes, one Node process has
   a bounded event-loop budget. The constant-arrival-rate scenario with
   `maxVUs: 200` simulates bursts beyond one instance's capacity.
   *Mitigation:* run the route on a horizontally scaled / serverless runtime and
   keep MongoDB connection pooling tuned (`maxPoolSize`).

## Recording results

When you run the suite, capture the k6/Artillery summary here and compare
against the targets above. If p95 latency exceeds 500 ms, start with bottleneck
#1 (indexes) — it has produced the largest wins in similar discovery endpoints.
