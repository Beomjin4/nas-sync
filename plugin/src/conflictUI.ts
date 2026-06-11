import { App, Modal, Notice, Setting, TFile } from "obsidian";
import type { ConflictInfo } from "./api";
import type NasSyncPlugin from "./main";

/** Lists unresolved conflicts; clicking one opens the resolve modal. */
export class ConflictListModal extends Modal {
  constructor(
    app: App,
    private plugin: NasSyncPlugin,
    private conflicts: ConflictInfo[],
  ) {
    super(app);
  }

  onOpen() {
    const { contentEl } = this;
    contentEl.createEl("h2", { text: "Sync conflicts" });

    if (this.conflicts.length === 0) {
      contentEl.createEl("p", { text: "No unresolved conflicts. 🎉" });
      return;
    }

    for (const c of this.conflicts) {
      new Setting(contentEl)
        .setName(c.path)
        .setDesc(`detected ${c.detected_at}`)
        .addButton((b) =>
          b.setButtonText("Resolve…").onClick(() => {
            this.close();
            void openResolveModal(this.app, this.plugin, c);
          }),
        );
    }
  }

  onClose() {
    this.contentEl.empty();
  }
}

async function openResolveModal(
  app: App,
  plugin: NasSyncPlugin,
  conflict: ConflictInfo,
) {
  try {
    const losingBuf = await plugin.api.getConflictFile(conflict.id);
    const losingText = safeDecode(losingBuf);

    let activeText = "(file not found locally)";
    const af = app.vault.getAbstractFileByPath(conflict.path);
    if (af instanceof TFile) {
      activeText = await app.vault.read(af);
    }

    new ConflictResolveModal(app, plugin, conflict, activeText, losingText).open();
  } catch (e) {
    new Notice(
      "Failed to load conflict: " + (e instanceof Error ? e.message : String(e)),
    );
  }
}

class ConflictResolveModal extends Modal {
  constructor(
    app: App,
    private plugin: NasSyncPlugin,
    private conflict: ConflictInfo,
    private activeText: string,
    private losingText: string,
  ) {
    super(app);
  }

  onOpen() {
    const { contentEl, modalEl } = this;
    modalEl.addClass("nas-sync-conflict-modal");
    contentEl.createEl("h2", { text: `Conflict: ${this.conflict.path}` });

    const grid = contentEl.createDiv({ cls: "nas-sync-conflict-grid" });
    grid.style.display = "grid";
    grid.style.gridTemplateColumns = "1fr 1fr";
    grid.style.gap = "8px";

    for (const [title, text] of [
      ["Current version (active)", this.activeText],
      [`Other device's version (${this.conflict.losing_device ?? "?"})`, this.losingText],
    ] as const) {
      const col = grid.createDiv();
      col.createEl("h4", { text: title });
      const pre = col.createEl("pre");
      pre.style.maxHeight = "40vh";
      pre.style.overflow = "auto";
      pre.style.whiteSpace = "pre-wrap";
      pre.style.border = "1px solid var(--background-modifier-border)";
      pre.style.padding = "8px";
      pre.setText(text);
    }

    const buttons = new Setting(contentEl);
    buttons.addButton((b) =>
      b
        .setButtonText("Keep current")
        .setCta()
        .onClick(() => void this.resolve("keep_active")),
    );
    buttons.addButton((b) =>
      b
        .setButtonText("Use other version")
        .onClick(() => void this.resolve("use_other")),
    );
    buttons.addButton((b) =>
      b
        .setButtonText("Keep both")
        .onClick(() => void this.resolve("keep_both")),
    );
  }

  private async resolve(choice: "keep_active" | "use_other" | "keep_both") {
    try {
      await this.plugin.api.resolveConflict(this.conflict.id, choice);
      if (choice === "use_other") {
        // The broadcast is suppressed for the resolver; pull explicitly.
        await this.plugin.engine.pullPath(this.conflict.path);
      }
      new Notice(`Resolved ${this.conflict.path} (${choice.replace("_", " ")})`);
      this.close();
      await this.plugin.refreshConflictBadge();
    } catch (e) {
      new Notice(
        "Resolve failed: " + (e instanceof Error ? e.message : String(e)),
      );
    }
  }

  onClose() {
    this.contentEl.empty();
  }
}

function safeDecode(buf: ArrayBuffer): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(buf);
  } catch {
    return `(binary content, ${buf.byteLength} bytes)`;
  }
}
