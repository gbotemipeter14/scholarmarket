import http from "k6/http";
import { check, sleep, group } from "k6";
import { Rate } from "k6/metrics";

// Load test for the marketplace search endpoint (GET /api/materials/search).
//
// Goal (issue #39): prove the route handles 100+ concurrent users and
// ~100 requests/second with an average response time under 500ms.
//
// Run:
//   export BASE_URL=http://localhost:3000
//   k6 run tests/load/materials-search.k6.js
//
// A pre-seeded MongoDB + the dev server (npm run dev) must be running first.
// The summary printed by k6 reports: http_reqs (throughput), http_req_duration
// (avg/p95/p99 = latency) and the failed-request rate (errorRate below).

const BASE_URL = __ENV.BASE_URL || "http://localhost:3000";

// A varied set of search queries so the cache is exercised but not every
// request is a unique miss (which would only measure raw MongoDB latency).
const QUERIES = [
  { q: "calculus" },
  { q: "machine learning" },
  { q: "organic chemistry" },
  { q: "world history" },
  { subject: "Mathematics" },
  { subject: "Physics", level: "University" },
  { category: "Presentation" },
  { q: "algebra", sortBy: "newest" },
  { q: "biology", licenseType: "Creative Commons" },
  { q: "economics", page: "2" },
];

// Tracks the share of non-2xx responses as the "error rate".
const errorRate = new Rate("error_rate");

export const options = {
  // Hold 100 VUs and drive a constant 100 requests/second at the endpoint,
  // ramping up so the system is not shocked on the first second.
  scenarios: {
    search_throughput: {
      executor: "constant-arrival-rate",
      rate: 100, // requests per second
      timeUnit: "1s",
      duration: "2m",
      preAllocatedVUs: 100,
      maxVUs: 200,
    },
    concurrency_spike: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 120 }, // 120 concurrent users
        { duration: "1m", target: 120 },
        { duration: "30s", target: 0 },
      ],
      gracefulRampDown: "10s",
    },
  },
  thresholds: {
    // Acceptance criteria for the issue.
    http_req_duration: ["p(95)<500", "p(99)<1000"],
    error_rate: ["rate<0.01"],
  },
};

function buildUrl(params) {
  const search = new URLSearchParams(params);
  return `${BASE_URL}/api/materials/search?${search.toString()}`;
}

export default function () {
  const params = QUERIES[Math.floor(Math.random() * QUERIES.length)];
  const url = buildUrl(params);

  const res = http.get(url, {
    headers: { Accept: "application/json" },
    tags: { name: "materials_search" },
  });

  const ok = check(res, {
    "status is 200": (r) => r.status === 200,
    "returns json": (r) => (r.headers["Content-Type"] || "").includes("application/json"),
    "has items array": (r) => {
      try {
        return Array.isArray(JSON.parse(r.body).items);
      } catch {
        return false;
      }
    },
  });

  errorRate.add(!ok);
  sleep(Math.random() * 0.5);
}
