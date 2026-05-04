import {
  App,
  FileSystemAdapter,
  Notice,
  Plugin,
  PluginSettingTab,
  Setting,
  TFile,
  debounce,
} from "obsidian";
import { exec } from "child_process";
import { access, constants } from "fs";

interface SharpTaskSettings {
  /** Path to the sharptask binary. Defaults to "sharptask" (PATH lookup). */
  sharptaskBin: string;
  /** Debounce delay in milliseconds before triggering a sync after a save. */
  debounceMs: number;
}

const DEFAULT_SETTINGS: SharpTaskSettings = {
  sharptaskBin: "sharptask",
  debounceMs: 500,
};

export default class SharpTaskPlugin extends Plugin {
  settings: SharpTaskSettings;
  private statusBarEl: HTMLElement;

  async onload() {
    await this.loadSettings();

    this.statusBarEl = this.addStatusBarItem();
    this.statusBarEl.setText("⚔️");
    this.statusBarEl.setAttr("title", "SharpTask: idle");

    const debouncedSync = debounce(
      (file: TFile) => this.syncFile(file),
      this.settings.debounceMs,
      true
    );

    this.registerEvent(
      this.app.vault.on("modify", (file) => {
        if (file instanceof TFile && file.extension === "md") {
          debouncedSync(file);
        }
      })
    );

    this.addSettingTab(new SharpTaskSettingTab(this.app, this));
  }

  private syncFile(file: TFile): void {
    if (!(this.app.vault.adapter instanceof FileSystemAdapter)) {
      // Mobile — child_process not available
      return;
    }

    const absPath = this.app.vault.adapter.getFullPath(file.path);
    const bin = this.settings.sharptaskBin;

    this.resolvebin(bin, (resolvedBin) => {
      if (!resolvedBin) {
        new Notice(`SharpTask: binary not found — "${bin}"`, 5000);
        this.statusBarEl.setText("⚔️ ✗");
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
          this.statusBarEl.setAttr("title", `SharpTask: error — ${msg}`);
          setTimeout(() => {
            this.statusBarEl.setText("⚔️");
            this.statusBarEl.setAttr("title", "SharpTask: idle");
          }, 5000);
          return;
        }

        const lines = stdout.trim().split("\n").filter(Boolean);
        // Each sync'd task prints "  - [x] Description ..." — show them all.
        const synced = lines.filter((l) => l.trim().startsWith("- "));
        if (synced.length > 0) {
          new Notice(`⚔️ Synced ${file.name}\n${synced.join("\n")}`, 4000);
        } else {
          new Notice(`⚔️ ${file.name} — no changes`, 2000);
        }

        this.statusBarEl.setText("⚔️");
        this.statusBarEl.setAttr("title", "SharpTask: idle");
      });
    });
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

    // Bare name: try PATH, then common Cargo/local install locations.
    const candidates = [
      bin,
      `${process.env.HOME}/.local/bin/sharptask`,
      `${process.env.HOME}/.cargo/bin/sharptask`,
    ];

    const tryNext = (i: number): void => {
      if (i >= candidates.length) {
        callback(null);
        return;
      }
      const candidate = candidates[i];
      // For bare names exec will find them via PATH, so just try directly.
      if (!candidate.startsWith("/")) {
        exec(`which "${candidate}"`, (err, out) => {
          if (!err && out.trim()) {
            callback(out.trim());
          } else {
            tryNext(i + 1);
          }
        });
        return;
      }
      access(candidate, constants.X_OK, (err) => {
        if (!err) callback(candidate);
        else tryNext(i + 1);
      });
    };

    tryNext(0);
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
      .setName("sharptask binary")
      .setDesc(
        'Path to the sharptask executable. Use a full path or leave as "sharptask" ' +
          "to auto-detect from PATH and ~/.local/bin / ~/.cargo/bin."
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
          "Increase if you find syncs firing mid-edit."
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
