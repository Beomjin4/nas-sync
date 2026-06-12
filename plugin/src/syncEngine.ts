import { App, Notice, TAbstractFile, TFile } from "obsidian";
import { NasApi } from "./api";
import { ExcludeMatcher } from "./exclude";
import type { SyncMessage } from "./sync";

const DEBOUNCE_MS = 5000;
const IGNORE_WINDOW_MS = 1500;
const RETRY_MIN_MS = 5000;
const RETRY_MAX_MS = 60000;

export interface SyncEngineState {
  /** path → server etag the client believes is current */
  etags: Record<string, string>;
  /** ops queued when the network was unavailable */
  queue: QueuedOp[];
  /**
   * path → local file mtime/size at the moment of the last successful
   * push or pull. Lets fullScan detect files changed while the plugin
   * wasn't running (external editors, app killed inside the debounce
   * window) using Obsidian's in-memory TFile.stat — no extra disk I/O.
   */
  fileMeta: Record<string, FileMeta>;
}

export interface FileMeta {
  m: number; // mtime (ms)
  s: number; // size (bytes)
}

export interface QueuedOp {
  path: string;
  action: "put" | "delete";
  etag: string | null;
  queuedAt: number;
}

export const DEFAULT_SYNC_STATE: SyncEngineState = {
  etags: {},
  queue: [],
  fileMeta: {},
};

/**
 * Coordinates push (vault → NAS) and pull (NAS → vault).
 *
 * Push: vault events are debounced per-path (5s); on flush we PUT with the
 * last-known etag for If-Match. Server's B+ policy means a mismatch isn't
 * fatal — the response carries `conflict` and we surface it. On network
 * failure the op is enqueued and retried with exponential backoff.
 *
 * Pull: SyncMessage routes (file_changed → GET → write, file_deleted →
 * delete). After writing, the path is added to `ignoredPaths` for
 * IGNORE_WINDOW_MS so the vault event we just triggered doesn't echo back.
 */
export class SyncEngine {
  private pendingTimers = new Map<string, number>();
  private ignoredPaths = new Map<string, number>();
  private etags = new Map<string, string>();
  private fileMeta = new Map<string, FileMeta>();
  private queue: QueuedOp[] = [];
  private retryTimer: number | null = null;
  private retryDelayMs = RETRY_MIN_MS;
  private excludes: ExcludeMatcher;

  constructor(
    private app: App,
    private api: NasApi,
    private persistState: (s: SyncEngineState) => Promise<void>,
    private getExcludePatterns: () => string[],
  ) {
    this.excludes = new ExcludeMatcher(getExcludePatterns());
  }

  reloadExcludes() {
    this.excludes = new ExcludeMatcher(this.getExcludePatterns());
  }

  load(state: SyncEngineState) {
    this.etags = new Map(Object.entries(state.etags || {}));
    this.fileMeta = new Map(Object.entries(state.fileMeta || {}));
    this.queue = [...(state.queue || [])];
    if (this.queue.length > 0) this.scheduleRetry(0);
  }

  dispose() {
    for (const t of this.pendingTimers.values()) window.clearTimeout(t);
    this.pendingTimers.clear();
    if (this.retryTimer !== null) {
      window.clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  // ---------- vault → NAS (push) ----------

  onCreate = (file: TAbstractFile) => {
    if (file instanceof TFile && !this.isIgnored(file.path)) {
      this.schedule(file.path, "put");
    }
  };

  onModify = (file: TAbstractFile) => {
    if (file instanceof TFile && !this.isIgnored(file.path)) {
      this.schedule(file.path, "put");
    }
  };

  onDelete = (file: TAbstractFile) => {
    if (file instanceof TFile && !this.isIgnored(file.path)) {
      this.schedule(file.path, "delete");
    }
  };

  onRename = (file: TAbstractFile, oldPath: string) => {
    if (!(file instanceof TFile)) return;
    if (!this.isIgnored(oldPath)) this.schedule(oldPath, "delete");
    if (!this.isIgnored(file.path)) this.schedule(file.path, "put");
  };

  private schedule(path: string, action: "put" | "delete") {
    if (this.excludes.isExcluded(path)) return;
    const existing = this.pendingTimers.get(path);
    if (existing !== undefined) window.clearTimeout(existing);
    const id = window.setTimeout(() => {
      this.pendingTimers.delete(path);
      this.flushOne(path, action).catch((e) => {
        console.error("[nas-sync] flush failed; queued for retry", path, e);
        this.enqueue({
          path,
          action,
          etag: this.etags.get(path) ?? null,
          queuedAt: Date.now(),
        });
      });
    }, DEBOUNCE_MS);
    this.pendingTimers.set(path, id);
  }

  private async flushOne(path: string, action: "put" | "delete") {
    await this.doOp({
      path,
      action,
      etag: this.etags.get(path) ?? null,
      queuedAt: Date.now(),
    });
    // Drop any prior queued entry for the same path — we just succeeded.
    if (this.queue.some((q) => q.path === path)) {
      this.queue = this.queue.filter((q) => q.path !== path);
      await this.persist();
    }
  }

  /** Execute one op against the server. Throws on network errors. */
  private async doOp(op: QueuedOp): Promise<void> {
    if (op.action === "delete") {
      if (!op.etag) {
        // We never had a remote version. Nothing to do.
        return;
      }
      const result = await this.api.deleteFile(op.path, op.etag);
      if (result === "stale") {
        new Notice(`${op.path}: server has newer version; restoring locally`);
        await this.pullFile(op.path);
      } else {
        this.etags.delete(op.path);
        this.fileMeta.delete(op.path);
        await this.persist();
      }
      return;
    }

    const af = this.app.vault.getAbstractFileByPath(op.path);
    if (!(af instanceof TFile)) return; // file disappeared
    const data = await this.app.vault.readBinary(af);
    const result = await this.api.putFile(op.path, data, op.etag ?? undefined);
    this.etags.set(op.path, result.etag);
    this.fileMeta.set(op.path, { m: af.stat.mtime, s: af.stat.size });
    if (result.conflict) {
      new Notice(`Conflict on ${op.path}; previous version preserved server-side`);
    }
    await this.persist();
  }

  // ---------- queue ----------

  private enqueue(op: QueuedOp) {
    this.queue = this.queue.filter((q) => q.path !== op.path);
    this.queue.push(op);
    void this.persist();
    this.scheduleRetry(this.retryDelayMs);
  }

  private scheduleRetry(delayMs: number) {
    if (this.retryTimer !== null) return;
    this.retryTimer = window.setTimeout(() => {
      this.retryTimer = null;
      void this.processQueue();
    }, delayMs);
  }

  private async processQueue() {
    if (this.queue.length === 0) {
      this.retryDelayMs = RETRY_MIN_MS;
      return;
    }
    const op = this.queue[0];
    try {
      await this.doOp(op);
      this.queue.shift();
      await this.persist();
      this.retryDelayMs = RETRY_MIN_MS;
      if (this.queue.length > 0) this.scheduleRetry(0);
    } catch (e) {
      console.warn("[nas-sync] queued op still failing", op.path, e);
      this.retryDelayMs = Math.min(this.retryDelayMs * 2, RETRY_MAX_MS);
      this.scheduleRetry(this.retryDelayMs);
    }
  }

  /** Called by the plugin when the WS reconnects, to flush sooner. */
  kickRetry() {
    if (this.queue.length > 0) this.scheduleRetry(0);
  }

  /**
   * Push every vault file the server doesn't know about yet (no cached etag).
   * Covers initial onboarding and files created while the plugin was off.
   * Note: files modified offline that already have an etag are pushed by the
   * normal modify event when the user next touches them, not by this scan.
   */
  fullScan(): number {
    let scheduled = 0;
    let adopted = false;
    for (const f of this.app.vault.getFiles()) {
      if (this.excludes.isExcluded(f.path)) continue;
      if (!this.etags.has(f.path)) {
        // Server has never seen this file.
        this.schedule(f.path, "put");
        scheduled += 1;
        continue;
      }
      const meta = this.fileMeta.get(f.path);
      if (!meta) {
        // Migration: synced before mtime tracking existed. Adopt the
        // current stat as the baseline without pushing.
        this.fileMeta.set(f.path, { m: f.stat.mtime, s: f.stat.size });
        adopted = true;
      } else if (meta.m !== f.stat.mtime || meta.s !== f.stat.size) {
        // Changed while the plugin wasn't watching (external editor,
        // app killed inside the debounce window, …).
        this.schedule(f.path, "put");
        scheduled += 1;
      }
    }
    if (adopted) void this.persist();
    return scheduled;
  }

  /**
   * Mirror of fullScan for the pull direction: fetch the server index and
   * download files we don't have. Used for first-time onboarding of a new
   * device and to catch changes broadcast while we were offline.
   *
   * Rules per server entry (excluded paths skipped):
   * - no local file               → pull
   * - local file, cached etag matches server  → in sync, skip
   * - local file, cached etag differs/missing → pull (server wins; local
   *   edits made while offline are the known reconcile gap)
   */
  async pullScan(): Promise<number> {
    const remote = await this.api.listFiles();
    let pulled = 0;
    for (const entry of remote) {
      if (this.excludes.isExcluded(entry.path)) continue;
      const local = this.app.vault.getAbstractFileByPath(entry.path);
      if (local instanceof TFile && this.etags.get(entry.path) === entry.etag) {
        continue;
      }
      try {
        await this.pullFile(entry.path);
        pulled += 1;
      } catch (e) {
        console.error("[nas-sync] pullScan failed for", entry.path, e);
      }
    }
    return pulled;
  }

  // ---------- NAS → vault (pull) ----------

  /** Explicit pull, used after resolving a conflict with `use_other`. */
  async pullPath(path: string) {
    await this.pullFile(path);
  }

  handleSyncMessage = async (msg: SyncMessage) => {
    if (msg.type === "hello") return;
    if ("path" in msg && this.excludes.isExcluded(msg.path)) return;
    if (msg.type === "file_changed") {
      await this.pullFile(msg.path);
    } else if (msg.type === "file_deleted") {
      await this.deleteLocal(msg.path);
    } else if (msg.type === "file_conflict") {
      new Notice(`Conflict on ${msg.path}`);
    }
  };

  private async pullFile(path: string) {
    const fetched = await this.api.getFile(path);
    if (!fetched) return;

    this.markIgnored(path);
    const af = this.app.vault.getAbstractFileByPath(path);
    if (af instanceof TFile) {
      await this.app.vault.modifyBinary(af, fetched.data);
    } else {
      await this.ensureParentDir(path);
      await this.app.vault.createBinary(path, fetched.data);
    }
    this.etags.set(path, fetched.etag);
    // TFile.stat may not reflect the write yet; ask the adapter directly.
    const st = await this.app.vault.adapter.stat(path);
    if (st) this.fileMeta.set(path, { m: st.mtime, s: st.size });
    await this.persist();
  }

  private async deleteLocal(path: string) {
    const af = this.app.vault.getAbstractFileByPath(path);
    if (af) {
      this.markIgnored(path);
      // vault.trash predates fileManager.trashFile and keeps us within
      // the declared minAppVersion; `true` = system trash.
      await this.app.vault.trash(af, true);
    }
    this.etags.delete(path);
    this.fileMeta.delete(path);
    await this.persist();
  }

  // ---------- helpers ----------

  private async ensureParentDir(path: string) {
    const idx = path.lastIndexOf("/");
    if (idx <= 0) return;
    const parent = path.substring(0, idx);
    if (!this.app.vault.getAbstractFileByPath(parent)) {
      await this.app.vault.createFolder(parent);
    }
  }

  private markIgnored(path: string) {
    this.ignoredPaths.set(path, Date.now() + IGNORE_WINDOW_MS);
  }

  private isIgnored(path: string): boolean {
    const expiry = this.ignoredPaths.get(path);
    if (expiry === undefined) return false;
    if (Date.now() < expiry) return true;
    this.ignoredPaths.delete(path);
    return false;
  }

  private async persist() {
    await this.persistState({
      etags: Object.fromEntries(this.etags),
      queue: [...this.queue],
      fileMeta: Object.fromEntries(this.fileMeta),
    });
  }
}
