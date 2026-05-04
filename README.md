![Image of a purple obsidian scalpel](sharptask.png)
# Sharptask

Manage your tasks with the precision of an obsidian scalpel.

___

Sharptask is a bridge between the excellent [Obsidian
Tasks](https://github.com/obsidian-tasks-group/obsidian-tasks) and
[taskwarrior](https://github.com/GothenburgBitFactory/taskwarrior). It searches through your entire
obsidian vault using [ripgrep](https://github.com/BurntSushi/ripgrep), parsing all markdown files it
finds. The operation depends on which command is issued. Currently, sharptask supports two commands:
tc-to-md and md-to-tc. These commands are intended to be used in tandem. When you have edited tasks
in taskwarrior, you should run tc-to-md. If you have edited files in obsidian, you should run
md-to-tc. Sharptask does not have the ability to merge edits in both sources, so ensuring it is run
after edites take place is important. 

In the future, instructions
for how to set up a taskwarrior hook and possibly development on an obsidian plugin will enable more
seemless syncing.

## MD to TC 

This mode will find all tasks in your vault, parse them, and either create taskwarrior
representations of them or update existing representations. Sharptask keeps track of which
taskwarrior task represents your task by embedding the taskwarrior UUID in your markdown. By
default, sharptask will use the obsidian link display text syntax to hide the UUID, replacing it
with a ⚔️emoji. This way you can easily tell which of your tasks are currently tracked in
taskwarrior! 

## TC to MD

This mode will find all tracked tasks (e.g. tasks with UUIDs) in your vault and update their
representation according to their current taskwarrior representation.

## Task Representation

Currently, sharptask supports the following Obsidian Task plugin features:

- Dates
    - Due
    - Scheduled
    - Start (implemented as the 'wait' date in TC)
    - Created
    - Completed
    - Canceled
- Priorities (mapped in the following manner)
    1. 🔺 maps to priority:H and the +next tag
    2. ⏫ maps to priority:H without the +next tag
    3. 🔼 maps to priority:M
    4. 🔽 and ⏬️ both map to priority:L
- Tags
    - Obsidian #tag tags are correctly mapped to TC
    - Taskwarrior does not allow '/' for tag hierarchy, so if you use hierarchical tags they will each be represented by their own individual tag in TC
    - Tags cannot contain spaces or any of these characters: !@#$%^&*(),.?":{}|<>
- Project
    - Projects are implemented using the 🔨 emoji. The entire text is captured as the project.
    - Hopefully we can get this added to the obsidian tasks plugin someday!

## Configuration

Sharptask looks for the following configuration file: ~/.sharptask/config.toml

These are the current configurations:

- vault_path: The default path to use for your vault when invoking sharptask
- tasknotes_path: Path to a folder of TaskNotes files (one `.md` per task). See [TaskNotes support](#tasknotes-obsidian-plugin-support) below.
- task_path: The path to your taskwarrior DB. Default: ~/.task/
- timezone: A [chrono_tz compatible string representation](https://docs.rs/chrono-tz/latest/chrono_tz/) of the timezone you want to use when parsing dates from obsidian. Default: the timezone your device is set to

```toml
# ~/.sharptask/config.toml
vault_path = "/Users/youruser/Documents/ObsidianVaults/MyMainVault"
tasknotes_path = "/Users/youruser/Documents/ObsidianVaults/MyMainVault/Tasks"
task_path = "/Users/youruser/.task"
timezone = "America/Chicago" # Or your local timezone
```

Each of these can be overridden at the command line. Use `--help` to learn more.

## TaskNotes (Obsidian Plugin) Support

Sharptask supports the [TaskNotes](https://github.com/callumalpass/tasknotes) Obsidian plugin, which
stores each task as a separate Markdown file with YAML frontmatter (rather than as inline checkboxes).

To enable this, point `--tasknotes` (or `tasknotes_path` in your config) at your TaskNotes folder:

```
sharptask --tasknotes ~/vault/Tasks md-to-tc
sharptask --tasknotes ~/vault/Tasks tc-to-md
```

### How it works

- **md-to-tc**: Each `.md` file in the folder is parsed. If the task has no `tc_uuid` field, a new
  Taskwarrior task is created and the UUID is written back into the frontmatter. Existing tracked
  tasks are synced to TC as normal.
- **tc-to-md**: For each TaskNotes file that has a `tc_uuid`, the corresponding TC task is checked.
  If the TC version differs, the file's YAML frontmatter is updated in place — the note body
  (everything after the closing `---`) is preserved unchanged.

### Supported fields

| TaskNotes frontmatter | Taskwarrior field |
|---|---|
| `title` | description |
| `status` (`todo`/`done`/`cancelled`) | status |
| `priority` (`low`/`medium`/`high`/`highest`) | priority + `next` tag |
| `due` | due |
| `scheduled` | scheduled |
| `start` | wait |
| `created` | created |
| `completed` / `cancelled` | end |
| `tags` | user tags |
| `project` | project |
| `tc_uuid` | UUID (written by sharptask) |

## Todo and Wishlist

- [ ] Clean up the code
    - [ ] Better document each section
    - [ ] Clean up messy logic in some places
- [ ] Improve testing
    - [ ] Add a more complete integration test suite
    - [ ] Add more testing for tc_to_md
- [ ] If tags are added in TC, format them more nicely in obsidian (maybe put them in paranthesis after the description?)
- [ ] Add more useful printout during operaiton
- [ ] Implement recurring Tasks
- [ ] Implement dependencies
- [ ] Automatically add nested list items in obsidian as annotations in TC
- [ ] Write obsidian plugin to automatically invoke with md-to-tc when tasks are edited in the markdown
- [ ] Add taskwarrior hooks to automatically invoke with tc-to-md when tasks are edited in taskwarrior
- [ ] Maintain indentation for tasks that are not at left-most level in document
