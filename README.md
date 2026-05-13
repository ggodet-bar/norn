# `splog`, a log viewer TUI with automatic tag categorization

## Why?

`splog` addresses a simple problem when going through massive log files: debugging sometimes requires that you focus on a sequence of messages printed by a single thread, agent, or tag, and push the rest of the log contents to the background.

`splog` does just that: its TUI creates panes dynamically for categories that appear several times in log line headers – ignoring timestamps and severities. This effectively creates automatic filters based on your app's log output.

## Example

![ExampleMain](https://raw.githubusercontent.com/ggodet-bar/splog/refs/heads/main/media/screen0.png)

This screenshot shows the `all` panel, which displays all the lines from the log files, like any file viewer would.
At the top of the screen, the categories that were automatically detected are listed next to the `all` panel – which is always present – as a list of tabs.
The tabs may be navigated either by pressing `Tab` or the corresponding index key when in the 0-9 range. Individual tabs may be hidden by pressing `Ctrl-X`.

![ExamplePanel](https://raw.githubusercontent.com/ggodet-bar/splog/refs/heads/main/media/screen1.png)

This second screenshot displays the `spark.SecurityManager` category/tab. This category was identified by `splog` because it appears several times in the log line headers (more details below).
Only a few lines from the full log appear, annotated with their line numbers from the original file.

## Navigation

Navigation inside the file works using the usual commands (pageUp/pageDown, arrow navigation, and vim-like navigation keys `hjkl`).

## Search

Searches may be run from any tab by pressing `/`. Once in the search mode, pressing `Ctrl-R` will handle the search query as a regex. If `splog` is executed in `follow` mode, searches will be updated with matches from the newly read lines.

Once a search is validated and active, pressing `c` will promote the search to a new category, which may then be navigated and searched. This is quite useful if you need to track events related to a specific identifier.

## How does categorization work?

`splog`'s categorization works with the assumption that the lines of the file it processes have a header part and a payload. A header part will typically contain timestamps, severity levels, and tags. These tags may be enclosed in brackets, parentheses, or separated from the rest of the header with "glue" characters such as dashes, colons, etc.

`splog` can handle a growing variety of timestamp and tag formats. Here are a few examples, taken from the test suite:

```
# Category: spark.SecurityManager
17/06/26 20:10:40 INFO spark.SecurityManager: Changing view acls to: yarn,curi

# Category: Main-Process
2026-05-02T09:43:45.729516 - INFO - Main-Process - payload

# Category: Strat
01/Jan/2026:03:45:49 +0100 DEBUG [Strat] Starting process round 0
```

A newly identified category will first enter a pending stage. It will only be promoted to a valid category and displayed as a tab if it occurs at least 12 times in the file, or if it appears at least 3 times within a window of 6 lines. This allows filtering out parsing noise and avoids cramming the tab pane with categories that are less relevant.

## Usage

```bash
Usage: splog [FILEPATH] [OPTIONS]

Options:
  -n, --max-lines N   retain at most N display rows; 0 = unlimited (default: 10000)
  -N, --no-line-numbers   hide the line-number gutter (file mode only)
  -f, --follow        keep reading the file as new lines are appended (file mode only)
  -V, --version       print version and exit
  -h, --help          show this help
```

## Installation

If you're a Rust programmer, `splog` can be installed with cargo.

Note that the minimum supported version of Rust for `splog` is 1.88.0.

```bash
cargo install splog
```

## Building

`splog` may be built from a recent-ish Rust installation (1.88.0 or newer).

To build `splog`:

```bash
$ git clone https://github.com/ggodet-bar/splog
$ cd splog
$ cargo build --release
$ ./target/release/splog --version
0.1.0
```
