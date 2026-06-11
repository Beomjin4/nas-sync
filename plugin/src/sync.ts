import type { NasSyncSettings } from "./settings";

export type SyncMessage =
  | { type: "hello"; device_id: string }
  | {
      type: "file_changed";
      path: string;
      etag: string;
      size: number;
      origin_device: string;
    }
  | {
      type: "file_deleted";
      path: string;
      origin_device: string;
    }
  | {
      type: "file_conflict";
      path: string;
      active_etag: string;
      losing_etag: string;
      conflict_id: string;
      origin_device: string;
    };

export class SyncClient {
  private ws: WebSocket | null = null;
  private reconnectTimer: number | null = null;
  private backoffMs = 1000;
  private intentionalClose = false;

  constructor(
    private getSettings: () => NasSyncSettings,
    private onMessage: (msg: SyncMessage) => void,
    private onConnected?: () => void,
  ) {}

  connect(): void {
    this.disconnect();
    this.intentionalClose = false;

    const s = this.getSettings();
    if (!s.serverUrl || !s.token) return;

    const wsUrl =
      s.serverUrl.replace(/^http/, "ws").replace(/\/$/, "") +
      "/sync?token=" +
      encodeURIComponent(s.token);

    try {
      this.ws = new WebSocket(wsUrl);
    } catch {
      this.scheduleReconnect();
      return;
    }

    this.ws.onopen = () => {
      this.backoffMs = 1000;
      console.log("[nas-sync] ws connected");
      try {
        this.onConnected?.();
      } catch (e) {
        console.warn("[nas-sync] onConnected handler threw", e);
      }
    };

    this.ws.onmessage = (ev) => {
      try {
        if (typeof ev.data === "string") {
          const msg = JSON.parse(ev.data) as SyncMessage;
          this.onMessage(msg);
        }
      } catch {
        console.warn("[nas-sync] bad ws message", ev.data);
      }
    };

    this.ws.onclose = () => {
      this.ws = null;
      console.log("[nas-sync] ws closed");
      if (!this.intentionalClose) this.scheduleReconnect();
    };

    this.ws.onerror = (e) => {
      console.warn("[nas-sync] ws error", e);
    };
  }

  disconnect(): void {
    this.intentionalClose = true;
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer !== null) return;
    const delay = Math.min(this.backoffMs, 30_000);
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      this.backoffMs = Math.min(this.backoffMs * 2, 30_000);
      this.connect();
    }, delay);
  }
}
