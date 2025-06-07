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

These are the current configuraitons:

- vault_path: The default path to use for your vault when invoking sharptask
- task_path: The path to your taskwarrior DB. Default: ~/.task/
- timezone: A [chrono_tz compatible string representation](https://docs.rs/chrono-tz/latest/chrono_tz/) of the timezone you want to use when parsing dates from obsidian. Default: the timezone your device is set to

```toml
# ~/.sharptask/config.toml
vault_path = "/Users/youruser/Documents/ObsidianVaults/MyMainVault"
task_path = "/Users/youruser/.task"
timezone = "America/Chicago" # Or your local timezone
```

Each of these can be overriden at the command line. Use `--help` to learn more.

## Todo and Wishlist

- [ ] Clean up the code
    - [ ] Better document each section
    - [ ] Clean up messy logic in some places
- [ ] Improve testing
    - [ ] Add a more complete integration test suite
    - [ ] Fix instability of the vault test
    - [ ] Add more testing for tc_to_md
- [ ] If tags are added in TC, format them more nicely in obsidian (maybe put them in paranthesis after the description?)
- [ ] Add more useful printout during operaiton
- [ ] Implement recurring Tasks
- [ ] Implement dependencies
- [ ] Automatically add nested list items in obsidian as annotations in TC
- [ ] Write obsidian plugin to automatically invoke with md-to-tc when tasks are edited in the markdown
- [ ] Add taskwarrior hooks to automatically invoke with tc-to-md when tasks are edited in taskwarrior
