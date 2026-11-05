# plannotator-tui-journal

A client of Plannotator's review journal, for terminal clients.
Plannotator keeps each review's comment threads in a journal on disk, by target:
the person's comments, replies and sub-threads, their resolutions,
the sends that carry them to the agent's session, and the agent's replies under them by id.
Its CLI, `plannotator threads`, reads and writes the journal for a client with no server of its own,
and the agent answers through the same CLI (`plannotator reply`).
This crate runs that CLI, synchronously, with no runtime and no network.

## Shape

- `Journal` is the seam: `Cli` runs Plannotator's CLI; an application's tests put a stand-in.
- `Cli::find` takes `plannotator` from a `PATH` value;
  `Cli::probe` checks that its `threads --help` names the client's actions,
  so a Plannotator without the review journal never gets a call,
  and asks it where its data directory is (`plannotator data-dir`).
- Every call is `plannotator threads <action> -`, its JSON object on stdin
  (a comment's words can be longer than a command line takes),
  naming the review by its target, `{ "kind": "last", "host", "session" }`.
  The CLI resolves its data directory itself, as the agent's `plannotator reply` does.
  A call that does not answer in time is stopped.
- Exit 0 is one JSON value on stdout; exit 1 a refusal, `<code>: <message>` on stderr
  (`Error::Refused`, its code `not_found`, `invalid`, `conflict` or `locked`);
  exit 2 a call the CLI does not read (`Error::Usage`).
- `send` takes the press's `IdempotencyKey`:
  the same press tried again goes under the same key,
  and the journal gives the first send back with its text instead of sending twice.
- The thread view is computed by Plannotator (`ReviewThreadView`, `v: 1`):
  a view in a newer contract is refused (`Error::Newer`), never guessed at.
- `stamp` reads the size and time of `reviews/<key>/journal.jsonl` under the data directory
  the probe was told, without running anything,
  so a client reads the view again only when the journal moved.

## Calls

| Call | CLI action | JSON beside `target` |
|---|---|---|
| `view` | `threads view` | none |
| `comment` | `threads comment` | `{ id, text, on, quote, type? }`, `{ id, text, re }` or `{ id, text, parent, quote, quote_offset? }` |
| `remove` | `threads remove` | `{ id }` |
| `resolve` | `threads resolve` or `threads reopen` | `{ id }` |
| `send` | `threads send` | `{ mode, ids?, idempotency_key }` |
| `delivered` | `threads delivered` | `{ sends }` |

## Tests and fixtures

`tests/fixtures/v1/` holds what Plannotator's own CLI answered for a review of two rounds,
written by `generate.sh <plannotator>` in a data directory of its own:
`view.json` (`threads view`) and `send.json` (`threads send`).
The client's tests run a stand-in program in a fresh folder under the temp dir
(on Unix; the parsing tests run everywhere): never a real Plannotator, never `~/.plannotator`.
