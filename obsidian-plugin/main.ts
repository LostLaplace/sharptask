import {
  App,
  FileSystemAdapter,
  Notice,
  Plugin,
  PluginSettingTab,
  Setting,
  TFile,
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
  /** Debounce delay in milliseconds before triggering a sync after a save. */
  debounceMs: number;
}

const DEFAULT_SETTINGS: SharpTaskSettings = {
  sharptaskBin: "sharptask",
  watchedFolder: "",
  debounceMs: 500,
};

export default class SharpTaskPlugin extends Plugin {
  settings: SharpTaskSettings;
  private statusBarEl: HTMLElement;

  /**
   * Per-file write-back suppression.
   *
   * When sharptask writes the frontmatter back to a file it just synced, that
   * write triggers another Obsidian "modify" event.  We track files that we
   * recently synced and ignore the next modify event they emit so we don't
   * enter an infinite loop.
   *
   * Maps vault-relative path → timeout handle that clears the entry.
   */
  private writeCooldown = new Map<string, ReturnType<typeof setTimeout>>();

  /** Per-file debounce timers (replaces Obsidian's debounce() helper). */
  private debounceTimers = new Map<string, ReturnType<typeof setTimeout>>();

  async onload() {
    await this.loadSettings();

    this.statusBarEl = this.addStatusBarItem();
    this.statusBarEl.setText("⚔️");
    this.statusBarEl.setAttr("title", "SharpTask: idle");

    this.registerEvent(
      this.app.vault.on("modify", (file) => {
        if (!(file instanceof TFile) || file.extension !== "md") return;

        // Ignore write-back events triggered by sharptask itself.
        if (this.writeCooldown.has(file.path)) return;

        // Only process files inside the configured watched folder.
        if (!this.isWatched(file)) return;

        // Per-file debounce: reset the timer on every rapid edit.
        const existing = this.debounceTimers.get(file.path);
        if (existing) clearTimeout(existing);
        const timer = setTimeout(() => {
          this.debounceTimers.delete(file.path);
          this.syncFile(file);
        }, this.settings.debounceMs);
        this.debounceTimers.set(file.path, timer);
      })
    );

    this.addSettingTab(new SharpTaskSettingTab(this.app, this));
  }

  onunload() {
    // Clean up all pending timers.
    for (const t of this.debounceTimers.values()) clearTimeout(t);
    for (const t of this.writeCooldown.values()) clearTimeout(t);
  }

  /** Returns true if `file` lives inside the configured watched folder. */
  private isWatched(file: TFile): boolean {
    const folder = this.settings.watchedFolder.trim().replace(/\/+$/, "");
    if (!folder) return true; // no filter configured — watch everything
    return (
      file.path === folder ||
      file.path.startsWith(folder + "/")
    );
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
          const msg = (stderr || error.message).trim();
          new Notice(`SharpTask ✗ ${file.name}:\n${msg}`, 6000);
          console.error("[sharptask]", msg);
          this.statusBarEl.setText("⚔️ ✗");
          setTimeout(() => {
            this.statusBarEl.setText("⚔️");
            this.statusBarEl.setAttr("title", "SharpTask: idle");
          }, 5000);
          // Keep cooldown active for 2s after failure so we don't retry immediately.
          this.setCooldown(file.path, 2000);
          return;
        }

        const lines = stdout.trim().split("\n").filter(Boolean);
        const synced = lines.filter((l) => l.trim().startsWith("- "));
        if (synced.length > 0) {
          new Notice(`⚔️ ${file.name}\n${synced.join("\n")}`, 3000);
        }
        // No notice for "no changes" — silent is better here.

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
      .setDesc(
        'Path to the sharptask executable. Leave as "sharptask" to ' +
          "auto-detect from PATH, ~/.local/bin, and ~/.cargo/bin."
      )
      .addText((text) =>
        text
          .setPlaceholder("sharptask")
          .setValue(this.plugin.settings.sharptaskBin)
          .onChange(async (value) => {
            this.plugin.settings.sharptaskBin = value.trim() || "sharptask";
            await this.plugin.saveSettings();
          })
      );

    new Setting(containerEl)
      .setName("Debounce delay (ms)")
      .setDesc(
        "How long to wait after the last keystroke before syncing. " +
          "Increase if syncs fire during rapid edits."
      )
      .addSlider((slider) =>
        slider
          .setLimits(100, 2000, 100)
          .setValue(this.plugin.settings.debounceMs)
          .setDynamicTooltip()
          .onChange(async (value) => {
            this.plugin.settings.debounceMs = value;
            await this.plugin.saveSettings();
          })
      );
  }
}
