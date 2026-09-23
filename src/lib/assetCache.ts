import type { Storage, StorageResponse } from "smplr";

/**
 * Remote audio assets (the GeneralUser GS SoundFont, Salamander piano
 * samples) are large and immutable, so under Tauri we persist them to the
 * app cache directory via Rust commands. Outside Tauri (plain `vite` in a
 * browser) this degrades to a normal fetch.
 */

export const isTauri = () =>
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invokeRaw<T>(
  cmd: string,
  args?: Record<string, unknown> | Uint8Array,
  headers?: Record<string, string>
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args, headers ? { headers } : undefined);
}

// Header values must be ASCII, so the URL is percent-encoded for both calls.
const cacheKey = (url: string) => encodeURIComponent(url);

async function readCache(url: string): Promise<ArrayBuffer | null> {
  try {
    const buf = await invokeRaw<ArrayBuffer>("asset_cache_get", {
      key: cacheKey(url),
    });
    return buf.byteLength > 0 ? buf : null;
  } catch {
    return null;
  }
}

async function writeCache(url: string, data: ArrayBuffer): Promise<void> {
  try {
    await invokeRaw<void>("asset_cache_put", new Uint8Array(data), {
      "x-asset-key": cacheKey(url),
    });
  } catch (e) {
    console.warn("asset cache write failed", url, e);
  }
}

/**
 * Make sure `url` is on disk (downloading it if needed) and return its cache
 * key, for assets that Rust reads directly. Tauri only.
 */
export async function ensureAssetCached(url: string): Promise<string> {
  const key = cacheKey(url);
  if (await invokeRaw<boolean>("asset_cache_has", { key })) return key;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  const buf = await res.arrayBuffer();
  await invokeRaw<void>("asset_cache_put", new Uint8Array(buf), {
    "x-asset-key": key,
  });
  return key;
}

/** Fetch a remote binary asset, serving from / populating the disk cache. */
export async function fetchCachedArrayBuffer(url: string): Promise<ArrayBuffer> {
  if (isTauri()) {
    const hit = await readCache(url);
    if (hit) return hit;
  }
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
  const buf = await res.arrayBuffer();
  if (isTauri()) void writeCache(url, buf.slice(0));
  return buf;
}

function bufferResponse(status: number, buf: ArrayBuffer): StorageResponse {
  return {
    status,
    arrayBuffer: async () => buf,
    text: async () => new TextDecoder().decode(buf),
    json: async () => JSON.parse(new TextDecoder().decode(buf)),
  };
}

/** smplr `Storage` backend that goes through the disk cache. */
export const diskCacheStorage: Storage = {
  async fetch(url: string): Promise<StorageResponse> {
    try {
      return bufferResponse(200, await fetchCachedArrayBuffer(url));
    } catch {
      return bufferResponse(404, new ArrayBuffer(0));
    }
  },
};
