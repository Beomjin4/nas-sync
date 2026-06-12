import { requestUrl } from "obsidian";
import type { NasSyncSettings } from "./settings";

export interface PairResponse {
  device_id: string;
  token: string;
}

export interface PutResult {
  etag: string;
  conflict: null | {
    id: string;
    previous_etag: string;
    stored_path: string;
  };
}

export interface FileFetch {
  data: ArrayBuffer;
  etag: string;
}

export interface RemoteFileEntry {
  path: string;
  etag: string;
  size_bytes: number;
  modified_at: string;
}

export interface ConflictInfo {
  id: string;
  path: string;
  active_etag: string;
  losing_etag: string;
  losing_device: string | null;
  detected_at: string;
}

export class NasApi {
  constructor(private getSettings: () => NasSyncSettings) {}

  async pair(
    code: string,
    name: string,
    platform: string,
  ): Promise<PairResponse> {
    const s = this.getSettings();
    if (!s.serverUrl) throw new Error("Server URL is not set");
    const r = await requestUrl({
      url: this.url("/devices/pair"),
      method: "POST",
      contentType: "application/json",
      body: JSON.stringify({
        pairing_code: code,
        name,
        platform,
      }),
      throw: false,
    });
    if (r.status >= 400) {
      throw new Error(`HTTP ${r.status}: ${r.text}`);
    }
    return parsePairResponse(r.json);
  }

  /** Returns null on 404. */
  async getFile(path: string): Promise<FileFetch | null> {
    const r = await requestUrl({
      url: this.url("/file/" + encodePath(path)),
      method: "GET",
      headers: this.authHeader(),
      throw: false,
    });
    if (r.status === 404) return null;
    if (r.status >= 400) {
      throw new Error(`GET ${path}: HTTP ${r.status}`);
    }
    const etagHeader = r.headers["etag"] || r.headers["ETag"] || "";
    return { data: r.arrayBuffer, etag: stripQuotes(etagHeader) };
  }

  /**
   * Upload a file. Omit `ifMatch` to declare a fresh create — if the server
   * disagrees (file already exists), the server's B+ policy still wins:
   * the response will include `conflict` and the caller should surface it.
   */
  async putFile(
    path: string,
    body: ArrayBuffer,
    ifMatch?: string,
  ): Promise<PutResult> {
    const headers = this.authHeader();
    if (ifMatch) headers["If-Match"] = `"${ifMatch}"`;
    const r = await requestUrl({
      url: this.url("/file/" + encodePath(path)),
      method: "PUT",
      headers,
      body,
      throw: false,
    });
    if (r.status >= 400) {
      throw new Error(`PUT ${path}: HTTP ${r.status} ${r.text}`);
    }
    return parsePutResult(r.json);
  }

  /**
   * Delete a file. `etag` must match the server's current version
   * (modify-wins policy on the server).
   */
  async deleteFile(path: string, etag: string): Promise<"deleted" | "not_found" | "stale"> {
    const r = await requestUrl({
      url: this.url("/file/" + encodePath(path)),
      method: "DELETE",
      headers: { ...this.authHeader(), "If-Match": `"${etag}"` },
      throw: false,
    });
    if (r.status === 204) return "deleted";
    if (r.status === 404) return "not_found";
    if (r.status === 412) return "stale";
    throw new Error(`DELETE ${path}: HTTP ${r.status}`);
  }

  /** Server-side index of every synced file. */
  async listFiles(): Promise<RemoteFileEntry[]> {
    const r = await requestUrl({
      url: this.url("/files"),
      method: "GET",
      headers: this.authHeader(),
      throw: false,
    });
    if (r.status >= 400) throw new Error(`GET /files: HTTP ${r.status}`);
    return parseArray(r.json, parseRemoteFileEntry);
  }

  // ---------- conflicts ----------

  async listConflicts(): Promise<ConflictInfo[]> {
    const r = await requestUrl({
      url: this.url("/conflicts"),
      method: "GET",
      headers: this.authHeader(),
      throw: false,
    });
    if (r.status >= 400) throw new Error(`GET /conflicts: HTTP ${r.status}`);
    return parseArray(r.json, parseConflictInfo);
  }

  /** Body of the losing (preserved) version of a conflict. */
  async getConflictFile(id: string): Promise<ArrayBuffer> {
    const r = await requestUrl({
      url: this.url(`/conflicts/${id}/file`),
      method: "GET",
      headers: this.authHeader(),
      throw: false,
    });
    if (r.status >= 400) {
      throw new Error(`GET /conflicts/${id}/file: HTTP ${r.status}`);
    }
    return r.arrayBuffer;
  }

  async resolveConflict(
    id: string,
    choice: "keep_active" | "use_other" | "keep_both",
  ): Promise<void> {
    const r = await requestUrl({
      url: this.url(`/conflicts/${id}/resolve`),
      method: "POST",
      contentType: "application/json",
      headers: this.authHeader(),
      body: JSON.stringify({ choice }),
      throw: false,
    });
    if (r.status >= 400) {
      throw new Error(`resolve ${id}: HTTP ${r.status} ${r.text}`);
    }
  }

  private authHeader(): Record<string, string> {
    const s = this.getSettings();
    return s.token ? { Authorization: `Bearer ${s.token}` } : {};
  }

  private url(path: string): string {
    const base = this.getSettings().serverUrl.replace(/\/$/, "");
    return base + path;
  }
}

function encodePath(p: string): string {
  return p.split("/").map(encodeURIComponent).join("/");
}

// ---------- response validation ----------
// requestUrl().json is untyped; validate shapes instead of blind casts.

function asObject(v: unknown, what: string): Record<string, unknown> {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    throw new Error(`unexpected server response (${what})`);
  }
  return v as Record<string, unknown>;
}

function str(o: Record<string, unknown>, key: string, what: string): string {
  const v = o[key];
  if (typeof v !== "string") {
    throw new Error(`unexpected server response (${what}.${key})`);
  }
  return v;
}

function num(o: Record<string, unknown>, key: string, what: string): number {
  const v = o[key];
  if (typeof v !== "number") {
    throw new Error(`unexpected server response (${what}.${key})`);
  }
  return v;
}

function parseArray<T>(v: unknown, item: (x: unknown) => T): T[] {
  if (!Array.isArray(v)) throw new Error("unexpected server response (array)");
  return v.map(item);
}

function parsePairResponse(v: unknown): PairResponse {
  const o = asObject(v, "pair");
  return { device_id: str(o, "device_id", "pair"), token: str(o, "token", "pair") };
}

function parsePutResult(v: unknown): PutResult {
  const o = asObject(v, "put");
  let conflict: PutResult["conflict"] = null;
  if (o.conflict !== null && o.conflict !== undefined) {
    const c = asObject(o.conflict, "put.conflict");
    conflict = {
      id: str(c, "id", "conflict"),
      previous_etag: str(c, "previous_etag", "conflict"),
      stored_path: str(c, "stored_path", "conflict"),
    };
  }
  return { etag: str(o, "etag", "put"), conflict };
}

function parseRemoteFileEntry(v: unknown): RemoteFileEntry {
  const o = asObject(v, "file");
  return {
    path: str(o, "path", "file"),
    etag: str(o, "etag", "file"),
    size_bytes: num(o, "size_bytes", "file"),
    modified_at: str(o, "modified_at", "file"),
  };
}

function parseConflictInfo(v: unknown): ConflictInfo {
  const o = asObject(v, "conflict");
  return {
    id: str(o, "id", "conflict"),
    path: str(o, "path", "conflict"),
    active_etag: str(o, "active_etag", "conflict"),
    losing_etag: str(o, "losing_etag", "conflict"),
    losing_device: typeof o.losing_device === "string" ? o.losing_device : null,
    detected_at: str(o, "detected_at", "conflict"),
  };
}

function stripQuotes(s: string): string {
  return s.replace(/^"(.*)"$/, "$1");
}
