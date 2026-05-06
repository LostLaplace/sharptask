import {
  App,
  FileSystemAdapter,
  FuzzySuggestModal,
  Notice,
  Plugin,
  PluginSettingTab,
  Setting,
  TFile,
  TFolder,
} from "obsidian";
import { exec } from "child_process";
import { access, constants } from "fs";
import * as path from "path";

interface SharpTaskSettings {
  /** Path to the sharptask binary. Defaults to "sharptask" (PATH lookup). */
  sharptaskBin: string;
  /**
   * Vault-relative folder path to watch (e.g. "TaskNotes/Tasks").
   * Only files under this folder will trigger a sync.
   * Leave empty to watch the entire vault (not recommended).
   */
  watchedFolder: string;
  /**
   * Newline-separated list of vault-relative folder paths to exclude from
   * sync (e.g. "Attachments\nArchive\nTemplates"). Files under these folders
   * will never trigger a sync even if they fall inside watchedFolder.
   */
  excludedFolders: string;
}

const DEFAULT_SETTINGS: SharpTaskSettings = {
  sharptaskBin: "sharptask",
  watchedFolder: "",
  excludedFolders: "",
};

export default class SharpTaskPlugin extends Plugin {
  settings: SharpTaskSettings;
  private statusBarEl: HTMLElement;

  /**
   * Files that have been edited since the last sync.
   * Synced when the user navigates away or the window loses focus.
   */
  private dirtyFiles = new Set<string>();

  /**
   * Per-file write-back suppression.
   *
   * When sharptask writes the frontmatter back to a file it just synced, that
   * write triggers another editor-change event.  We track files that we
   * recently synced and ignore editor-change events they emit during the
   * cooldown window so we don't mark them dirty again immediately.
   *
   * Maps vault-relative path → timeout handle that clears the entry.
   */
  private writeCooldown = new Map<string, ReturnType<typeof setTimeout>>();

  /** The file that was active just before the current active-leaf-change. */
  private lastActiveFile: TFile | null = null;

  async onload() {
    await this.loadSettings();

    this.statusBarEl = this.addStatusBarItem();
    this.statusBarEl.setText("⚔️");
    this.statusBarEl.setAttr("title", "SharpTask: idle");

    // Seed lastActiveFile so the first tab-switch can sync if needed.
    this.app.workspace.onLayoutReady(() => {
      this.lastActiveFile = this.app.workspace.getActiveFile();
    });

    // Mark a file dirty whenever the user edits it.
    this.registerEvent(
      this.app.workspace.on("editor-change", (_editor, info) => {
        const file = info.file;
        if (!(file instanceof TFile) || file.extension !== "md") return;
        if (this.writeCooldown.has(file.path)) return; // our own write-back
        if (this.isWatched(file)) this.dirtyFiles.add(file.path);
      })
    );

    // For files modified outside an open editor (e.g. task modals, hook
    // write-backs from TW), sync immediately on the modify event — but only
    // if the file is NOT currently open in any editor leaf (open files are
    // handled by the blur/leaf-change path above to avoid mid-typing syncs).
    this.registerEvent(
      this.app.vault.on("modify", (file) => {
        if (!(file instanceof TFile) || file.extension !== "md") return;
        if (this.writeCooldown.has(file.path)) return;
        if (!this.isWatched(file)) return;
        if (this.isFileOpen(file)) return; // handled by editor-change path
        this.syncFile(file);
      })
    );

    // Sync the previously active file when the user switches tabs/panes.
    this.registerEvent(
      this.app.workspace.on("active-leaf-change", (leaf) => {
        this.syncDirtyFile(this.lastActiveFile);

        const view = leaf?.view as { file?: unknown } | null;
        const next = view?.file;
        this.lastActiveFile = next instanceof TFile ? next : null;
      })
    );

    // Sync when the user switches to another application.
    this.registerDomEvent(window, "blur", () => {
      this.syncDirtyFile(this.lastActiveFile);
    });

    this.addSettingTab(new SharpTaskSettingTab(this.app, this));
  }

  onunload() {
    for (const t of this.writeCooldown.values()) clearTimeout(t);
  }

  /** Sync `file` if it is in the dirty set; no-op otherwise. */
  private syncDirtyFile(file: TFile | null): void {
    if (!file || !this.dirtyFiles.has(file.path)) return;
    this.dirtyFiles.delete(file.path);
    this.syncFile(file);
  }

  /** Returns true if `file` is currently open in any editor leaf. */
  private isFileOpen(file: TFile): boolean {
    return this.app.workspace.getLeavesOfType("markdown").some((leaf) => {
      const view = leaf.view as { file?: unknown };
      return view.file === file;
    });
  }

  /** Returns true if `file` lives inside the configured watched folder
   *  and is not under any excluded folder. */
  private isWatched(file: TFile): boolean {
    const folder = this.settings.watchedFolder.trim().replace(/\/+$/, "");
    if (folder && file.path !== folder && !file.path.startsWith(folder + "/")) {
      return false;
    }

    const excluded = this.settings.excludedFolders
      .split("\n")
      .map((s) => s.trim().replace(/\/+$/, ""))
      .filter((s) => s.length > 0);
    for (const ex of excluded) {
      if (file.path === ex || file.path.startsWith(ex + "/")) return false;
    }

    return true;
  }

  private syncFile(file: TFile): void {
    if (!(this.app.vault.adapter instanceof FileSystemAdapter)) return;

    const absPath = this.app.vault.adapter.getFullPath(file.path);
    const bin = this.settings.sharptaskBin;

    // Suppress the write-back modify event before we even call exec, so any
    // file write that happens during the sync is ignored.  The cooldown is
    // extended again once exec returns.
    this.setCooldown(file.path, 5000);

    this.resolvebin(bin, (resolvedBin) => {
      if (!resolvedBin) {
        new Notice(`SharpTask: binary not found — "${bin}"`, 5000);
        this.statusBarEl.setText("⚔️ ✗");
        this.clearCooldown(file.path);
        return;
      }

      this.statusBarEl.setText("⚔️ …");
      this.statusBarEl.setAttr("title", `SharpTask: syncing ${file.name}`);

      const cmd = `"${resolvedBin}" --file "${absPath}" md-to-tc`;

      exec(cmd, (error, stdout, stderr) => {
        if (error) {
          // sharptask exits non-zero; stderr has the useful message.
          const msg = (stderr || error.message).trim().split("\n")[0];
          new Notice(`⚔️ ✗ ${file.name}: ${msg}`, 6000);
          console.error("[sharptask]", stderr || error.message);
          this.statusBarEl.setText("⚔️ ✗");
          setTimeout(() => {
            this.statusBarEl.setText("⚔️");
            this.statusBarEl.setAttr("title", "SharpTask: idle");
          }, 5000);
          this.setCooldown(file.path, 2000);
          return;
        }

        // sharptask prints "No changes" (with ANSI colors) when tc and the
        // file are already in sync.  Anything else means a new task was
        // created or an existing task was updated — show a brief notice.
        if (!stdout.includes("No changes")) {
          new Notice(`⚔️ Synced: ${file.name}`, 2500);
        }
        // Silent when nothing changed.

        this.statusBarEl.setText("⚔️");
        this.statusBarEl.setAttr("title", "SharpTask: idle");

        // Keep cooldown for 2s after exec returns to absorb the write-back event
        // that Obsidian fires when sharptask rewrites the frontmatter.
        this.setCooldown(file.path, 2000);
      });
    });
  }

  private setCooldown(vaultPath: string, ms: number): void {
    const existing = this.writeCooldown.get(vaultPath);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(
      () => this.writeCooldown.delete(vaultPath),
      ms
    );
    this.writeCooldown.set(vaultPath, timer);
  }

  private clearCooldown(vaultPath: string): void {
    const t = this.writeCooldown.get(vaultPath);
    if (t) clearTimeout(t);
    this.writeCooldown.delete(vaultPath);
  }

  /**
   * Resolve the sharptask binary path, checking common install locations when
   * the raw value is a bare name (not an absolute path).
   */
  private resolvebin(
    bin: string,
    callback: (resolved: string | null) => void
  ): void {
    if (bin.startsWith("/") || bin.startsWith("~")) {
      const expanded = bin.replace(/^~/, process.env.HOME ?? "~");
      access(expanded, constants.X_OK, (err) =>
        callback(err ? null : expanded)
      );
      return;
    }

    const candidates = [
      `${process.env.HOME}/.local/bin/sharptask`,
      `${process.env.HOME}/.cargo/bin/sharptask`,
    ];

    exec(`which "${bin}"`, (err, out) => {
      if (!err && out.trim()) {
        callback(out.trim());
        return;
      }
      const tryNext = (i: number): void => {
        if (i >= candidates.length) { callback(null); return; }
        access(candidates[i], constants.X_OK, (e) => {
          if (!e) callback(candidates[i]);
          else tryNext(i + 1);
        });
      };
      tryNext(0);
    });
  }

  async loadSettings() {
    this.settings = Object.assign({}, DEFAULT_SETTINGS, await this.loadData());
  }

  async saveSettings() {
    await this.saveData(this.settings);
  }
}

class SharpTaskSettingTab extends PluginSettingTab {
  plugin: SharpTaskPlugin;

  constructor(app: App, plugin: SharpTaskPlugin) {
    super(app, plugin);
    this.plugin = plugin;
  }

  display(): void {
    const { containerEl } = this;
    containerEl.empty();
    containerEl.createEl("h2", { text: "SharpTask Sync" });

    new Setting(containerEl)
      .setName("TaskNotes folder")
      .setDesc(
        "Vault-relative path to the folder containing your TaskNotes " +
          "(e.g. \"TaskNotes/Tasks\"). Only files in this folder trigger a sync. " +
          "Leave empty to watch the entire vault (not recommended)."
      )
      .addText((text) =>
        text
          .setPlaceholder("TaskNotes/Tasks")
          .setValue(this.plugin.settings.watchedFolder)
          .onChange(async (value) => {
            this.plugin.settings.watchedFolder = value.trim();
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("sharptask binary")

    new Setting(containerEl)
      .setName("Excluded folders")
      .setDesc(
        "Folders to exclude from sync. Files inside these folders will " +
          "never trigger a sync."
      );

    const excludedList = containerEl.createDiv("sharptask-excluded-list");
    const renderExcluded = () => {
      excludedList.empty();
      const folders = this.plugin.settings.excludedFolders
        .split("\n")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);

      for (const folder of folders) {
        const row = excludedList.createDiv("sharptask-excluded-row");
        row.createSpan({ text: folder, cls: "sharptask-excluded-label" });
        const removeBtn = row.createEl("button", { text: "✕" });
        removeBtn.onclick = async () => {
          const updated = folders.filter((f) => f !== folder);
          this.plugin.settings.excludedFolders = updated.join("\n");
          await this.plugin.saveSettings();
          renderExcluded();
        };
      }

      new Setting(excludedList)
        .addButton((btn) =>
          btn.setButtonText("Add folder").onClick(() => {
            new FolderSuggestModal(this.app, async (folder) => {
              const current = this.plugin.settings.excludedFolders
                .split("\n")
                .map((s) => s.trim())
                .filter((s) => s.length > 0);
              if (!current.includes(folder.path)) {
                current.push(folder.path);
                this.plugin.settings.excludedFolders = current.join("\n");
                await this.plugin.saveSettings();
              }
              renderExcluded();
            }).open();
          })
        );
    };
    renderExcluded();
  }
}

class FolderSuggestModal extends FuzzySuggestModal<TFolder> {
  private onChoose: (folder: TFolder) => void;

  constructor(app: App, onChoose: (folder: TFolder) => void) {
    super(app);
    this.onChoose = onChoose;
    this.setPlaceholder("Type to search folders…");
  }

  getItems(): TFolder[] {
    const folders: TFolder[] = [];
    const recurse = (folder: TFolder) => {
      folders.push(folder);
      for (const child of folder.children) {
        if (child instanceof TFolder) recurse(child);
      }
    };
    recurse(this.app.vault.getRoot());
    return folders.slice(1); // exclude root
  }

  getItemText(folder: TFolder): string {
    return folder.path;
  }

  onChooseItem(folder: TFolder, _evt: MouseEvent | KeyboardEvent): void {
    this.onChoose(folder);
  }
}
