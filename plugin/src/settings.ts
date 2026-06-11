import { App, Notice, Platform, PluginSettingTab, Setting } from "obsidian";
import type NasSyncPlugin from "./main";

export interface NasSyncSettings {
  serverUrl: string;
  pairingCode: string;
  deviceName: string;
  deviceId: string | null;
  token: string | null;
  /** One glob per entry; matching vault paths are never pushed nor pulled. */
  excludePatterns: string[];
}

export const DEFAULT_SETTINGS: NasSyncSettings = {
  serverUrl: "",
  pairingCode: "",
  deviceName: "",
  deviceId: null,
  token: null,
  excludePatterns: [],
};

export class NasSyncSettingTab extends PluginSettingTab {
  plugin: NasSyncPlugin;

  constructor(app: App, plugin: NasSyncPlugin) {
    super(app, plugin);
    this.plugin = plugin;
  }

  display(): void {
    const { containerEl } = this;
    containerEl.empty();

    new Setting(containerEl)
      .setName("Server URL")
      .setDesc("Example: http://192.168.1.10:8080")
      .addText((t) =>
        t
          .setPlaceholder("http://...")
          .setValue(this.plugin.settings.serverUrl)
          .onChange(async (v) => {
            this.plugin.settings.serverUrl = v.trim().replace(/\/$/, "");
            await this.plugin.saveSettings();
          }),
      );

    new Setting(containerEl)
      .setName("Admin console")
      .setDesc(
        "Activity log, trash restore, conflict resolution, device management, file search — served by the NAS.",
      )
      .addButton((b) =>
        b.setButtonText("Open admin console").onClick(() => {
          const base = this.plugin.settings.serverUrl;
          if (!base) {
            new Notice("Set the Server URL first.");
            return;
          }
          window.open(base + "/admin", "_blank");
        }),
      );

    new Setting(containerEl)
      .setName("Excluded paths")
      .setDesc(
        "One glob per line. Matching files are never synced in either direction. ** spans folders.",
      )
      .addTextArea((t) => {
        t.inputEl.rows = 4;
        t.setValue(this.plugin.settings.excludePatterns.join("\n")).onChange(
          async (v) => {
            this.plugin.settings.excludePatterns = v
              .split("\n")
              .map((s) => s.trim())
              .filter((s) => s.length > 0);
            await this.plugin.saveSettings();
            this.plugin.engine.reloadExcludes();
          },
        );
      });

    new Setting(containerEl)
      .setName("Device name")
      .setDesc("Shown in the NAS admin console.")
      .addText((t) =>
        t
          .setPlaceholder("e.g. MacBook")
          .setValue(this.plugin.settings.deviceName)
          .onChange(async (v) => {
            this.plugin.settings.deviceName = v;
            await this.plugin.saveSettings();
          }),
      );

    if (!this.plugin.settings.token) {
      new Setting(containerEl)
        .setName("Pairing code")
        .setDesc(
          "Set on the server via ONS_PAIRING_CODE. Discarded after successful pairing.",
        )
        .addText((t) =>
          t
            .setValue(this.plugin.settings.pairingCode)
            .onChange((v) => {
              this.plugin.settings.pairingCode = v;
            }),
        );

      new Setting(containerEl).addButton((b) =>
        b
          .setButtonText("Pair this device")
          .setCta()
          .onClick(async () => {
            try {
              const platform = detectPlatform();
              const r = await this.plugin.api.pair(
                this.plugin.settings.pairingCode,
                this.plugin.settings.deviceName || "unnamed",
                platform,
              );
              this.plugin.settings.deviceId = r.device_id;
              this.plugin.settings.token = r.token;
              this.plugin.settings.pairingCode = "";
              await this.plugin.saveSettings();
              new Notice("Paired with NAS");
              this.plugin.sync.connect();
              this.display();
            } catch (e) {
              const msg = e instanceof Error ? e.message : String(e);
              new Notice("Pairing failed: " + msg);
            }
          }),
      );
    } else {
      new Setting(containerEl)
        .setName("Paired")
        .setDesc(`Device ID: ${this.plugin.settings.deviceId}`)
        .addButton((b) =>
          b
            .setButtonText("Unpair")
            .setWarning()
            .onClick(async () => {
              this.plugin.settings.deviceId = null;
              this.plugin.settings.token = null;
              await this.plugin.saveSettings();
              this.plugin.sync.disconnect();
              new Notice("Unpaired");
              this.display();
            }),
        );
    }
  }
}

function detectPlatform(): string {
  if (Platform.isAndroidApp) return "android";
  if (Platform.isIosApp) return "ios";
  if (Platform.isMacOS) return "macos";
  if (Platform.isWin) return "windows";
  if (Platform.isLinux) return "linux";
  return "unknown";
}
