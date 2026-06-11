import { Notice, Plugin } from "obsidian";
import { NasApi } from "./api";
import { ConflictListModal } from "./conflictUI";
import {
  DEFAULT_SETTINGS,
  NasSyncSettingTab,
  NasSyncSettings,
} from "./settings";
import { SyncClient, SyncMessage } from "./sync";
import {
  DEFAULT_SYNC_STATE,
  SyncEngine,
  SyncEngineState,
} from "./syncEngine";

interface PluginData {
  settings: NasSyncSettings;
  syncState: SyncEngineState;
}

export default class NasSyncPlugin extends Plugin {
  settings!: NasSyncSettings;
  syncState!: SyncEngineState;
  api!: NasApi;
  sync!: SyncClient;
  engine!: SyncEngine;
  private statusBar!: HTMLElement;

  async onload() {
    await this.loadSettings();

    this.api = new NasApi(() => this.settings);
    this.engine = new SyncEngine(
      this.app,
      this.api,
      async (s) => {
        this.syncState = s;
        await this.saveSettings();
      },
      () => this.settings.excludePatterns,
    );
    this.engine.load(this.syncState);

    this.sync = new SyncClient(
      () => this.settings,
      (msg) => this.handleMessage(msg),
      () => {
        this.engine.kickRetry();
        const n = this.engine.fullScan();
        if (n > 0) new Notice(`NAS Sync: uploading ${n} new file(s)`);
        void this.engine.pullScan().then((pulled) => {
          if (pulled > 0) new Notice(`NAS Sync: downloaded ${pulled} file(s)`);
        });
        void this.refreshConflictBadge();
      },
    );

    this.statusBar = this.addStatusBarItem();
    this.statusBar.addClass("mod-clickable");
    this.statusBar.onClickEvent(() => void this.openConflictList());
    this.updateBadge(0);

    this.addSettingTab(new NasSyncSettingTab(this.app, this));

    this.registerEvent(this.app.vault.on("create", this.engine.onCreate));
    this.registerEvent(this.app.vault.on("modify", this.engine.onModify));
    this.registerEvent(this.app.vault.on("delete", this.engine.onDelete));
    this.registerEvent(this.app.vault.on("rename", this.engine.onRename));

    this.addCommand({
      id: "reconnect",
      name: "Reconnect to NAS",
      callback: () => {
        if (!this.settings.token) {
          new Notice("Not paired. Open Settings → NAS Sync.");
          return;
        }
        this.sync.connect();
      },
    });

    this.addCommand({
      id: "conflicts",
      name: "Show sync conflicts",
      callback: () => void this.openConflictList(),
    });

    this.addCommand({
      id: "push-all",
      name: "Push all unsynced files now",
      callback: () => {
        const n = this.engine.fullScan();
        new Notice(
          n > 0 ? `Scheduled ${n} file(s) for upload` : "Everything already synced",
        );
      },
    });

    if (this.settings.token) this.sync.connect();
  }

  onunload() {
    this.sync?.disconnect();
    this.engine?.dispose();
  }

  private handleMessage(msg: SyncMessage) {
    void this.engine.handleSyncMessage(msg);
    if (msg.type === "file_conflict") {
      void this.refreshConflictBadge();
    }
  }

  async openConflictList() {
    if (!this.settings.token) {
      new Notice("Not paired.");
      return;
    }
    try {
      const conflicts = await this.api.listConflicts();
      new ConflictListModal(this.app, this, conflicts).open();
    } catch (e) {
      new Notice(
        "Failed to list conflicts: " +
          (e instanceof Error ? e.message : String(e)),
      );
    }
  }

  async refreshConflictBadge() {
    if (!this.settings.token) return;
    try {
      const conflicts = await this.api.listConflicts();
      this.updateBadge(conflicts.length);
    } catch {
      // Offline etc. — leave the badge as-is.
    }
  }

  private updateBadge(count: number) {
    this.statusBar.setText(count > 0 ? `⚠ ${count} conflicts` : "NAS ✓");
    this.statusBar.toggleClass("nas-sync-has-conflicts", count > 0);
  }

  async loadSettings() {
    const raw = (await this.loadData()) as Partial<PluginData> | null;
    this.settings = { ...DEFAULT_SETTINGS, ...(raw?.settings ?? {}) };
    if (!raw?.settings?.excludePatterns) {
      this.settings.excludePatterns = [
        `${this.app.vault.configDir}/**`,
        ".trash/**",
      ];
    }
    this.syncState = { ...DEFAULT_SYNC_STATE, ...(raw?.syncState ?? {}) };
  }

  async saveSettings() {
    await this.saveData({
      settings: this.settings,
      syncState: this.syncState,
    } satisfies PluginData);
  }
}
