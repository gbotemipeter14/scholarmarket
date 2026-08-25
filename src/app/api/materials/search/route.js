// Resolves: Implement the dedicated marketplace search endpoint used by the
// discovery UI. Mirrors the discovery query builder used by /api/market-materials
// but is exposed at its own stable path so it can be load tested and cached
// independently.
export const dynamic = "force-dynamic";

import { NextResponse } from "next/server";
import { auditLog } from "@/lib/api/audit";
import { withApiHardening } from "@/lib/api/hardening";
import { parsePagination } from "@/lib/api/validation";
import { buildMarketplaceDiscoveryQuery, buildMarketplaceSort } from "@/lib/backend/marketplaceDiscovery";
import { getDb } from "@/lib/mongodb";
import { ObjectId } from "mongodb";
import { cacheGet, cacheSet } from "@/lib/cache/redis";

export const runtime = "nodejs";

function sanitizeMaterial(doc) {
  if (!doc) return doc;
  const { storageKey, fileUrl, metadataUrl, ...safe } = doc;
  const averageScore = Number(safe.averageScore ?? safe.rating ?? 0) || 0;
  const feedbackCount = Number(safe.feedbackCount ?? safe.reviewsCount ?? 0) || 0;

  return {
    ...safe,
    averageScore,
    rating: averageScore,
    feedbackCount,
    reviewsCount: feedbackCount,
    userAddress: safe.userAddress ?? safe.ownerAddress ?? null,
  };
}

// GET /api/materials/search?q=...&subject=...&category=...&level=...&creator=...&licenseType=...&sortBy=...&page=...&pageSize=...
// Search is the single most heavily used discovery route, so it has its own
// endpoint, aggressive caching, and a generous rate limit.
export async function GET(request) {
  return withApiHardening(
    request,
    { route: "materials-search", rateLimit: { limit: 200, windowMs: 60_000 } },
    async () => {
      try {
        const db = await getDb();
        const url = new URL(request.url);

        // The discovery query builder keys off `search`; accept either `q` or `search`.
        const params = new URLSearchParams(url.searchParams.toString());
        if (!params.has("search")) {
          const q = params.get("q");
          if (q) params.set("search", q);
        }

        const { page, pageSize } = parsePagination(params);
        const cacheKey = `materials-search:${params.toString()}`;
        const cached = await cacheGet(cacheKey);
        if (cached) {
          return NextResponse.json(cached, { status: 200 });
        }

        const query = buildMarketplaceDiscoveryQuery(params);
        const sort = buildMarketplaceSort(params.get("sortBy"));

        const total = await db.collection("materials").countDocuments(query);
        const items = await db
          .collection("materials")
          .find(query)
          .sort(sort)
          .skip((page - 1) * pageSize)
          .limit(pageSize)
          .toArray();

        const normalized = items.map(sanitizeMaterial);
        const totalPages = Math.max(1, Math.ceil(total / pageSize));
        const payload = { items: normalized, page, pageSize, total, totalPages };

        await cacheSet(cacheKey, payload, 300);
        return NextResponse.json(payload, { status: 200 });
      } catch (err) {
        if (err.name === "ValidationError") throw err;
        auditLog({ event: "materials_search_failed", route: "materials-search", method: "GET", status: 500, reason: err.message });
        return NextResponse.json({ error: "Server error" }, { status: 500 });
      }
    }
  );
}
