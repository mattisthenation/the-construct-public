# The Construct — Activity & Stats Access

The Construct records every pipeline run and its lifecycle events in a local SQLite
database, `construct.db`, in the working directory where `entertheconstruct` runs.
A future monitoring app (or a quick `ssh + sqlite3` session) can read it directly.
**This data is read-only for outside consumers — never write to `construct.db`
from another process while The Construct is running.**

## Schema

### `runs` — one row per pipeline run
| column      | type | meaning |
|-------------|------|---------|
| `id`        | TEXT PK | run UUID (also stamped into the note as `construct_run_id`) |
| `rule`      | TEXT | pipeline/rule that owns the run (e.g. `research`, `tag`, `inbox`) |
| `agent`     | TEXT | agent name (e.g. `Scout`, `Librarian`) |
| `note_path` | TEXT | absolute path of the note (or generated day note) |
| `status`    | TEXT | `queued`/`running`/`researching`/`review`/`accepted`/`rejected`/`done`/`error` |
| `error`     | TEXT | error message when `status = error`, else NULL |
| `created_at`| TEXT | UTC datetime the run was created |
| `updated_at`| TEXT | UTC datetime of the last status change |

### `run_events` — append-only timeline per run
| column     | type | meaning |
|------------|------|---------|
| `id`       | INTEGER PK AUTOINCREMENT | event sequence id |
| `run_id`   | TEXT FK→runs.id | the run this event belongs to |
| `stage`    | TEXT | pipeline stage (`claim`, `summarize`, `write_back`, `error`, …) |
| `event`    | TEXT | short event label (`queued`, `done`, `review`, `failed`, …) |
| `payload`  | TEXT | JSON blob with stage-specific detail |
| `ts`       | TEXT | UTC datetime of the event |

### `schedule_state` — last-run bookkeeping for scheduled jobs
| column     | type | meaning |
|------------|------|---------|
| `job`      | TEXT PK | scheduled job name (e.g. `daily_summary`) |
| `last_run` | TEXT | RFC3339 timestamp of the last successful run |

## Example read-only queries

Recent activity feed (latest events with their run context):
```sql
SELECT e.ts, r.rule, r.agent, e.stage, e.event, r.note_path
FROM run_events e JOIN runs r ON r.id = e.run_id
ORDER BY e.id DESC
LIMIT 50;
```

Runs grouped by status:
```sql
SELECT status, COUNT(*) AS n FROM runs GROUP BY status ORDER BY n DESC;
```

Runs grouped by pipeline and day:
```sql
SELECT date(created_at) AS day, rule, COUNT(*) AS n
FROM runs GROUP BY day, rule ORDER BY day DESC, n DESC;
```

Notes currently awaiting human review:
```sql
SELECT note_path, rule, agent, updated_at
FROM runs WHERE status = 'review' ORDER BY updated_at DESC;
```

Quick CLI peek over SSH:
```sh
ssh my-host "sqlite3 -readonly ~/path/to/construct.db \
  'SELECT status, COUNT(*) FROM runs GROUP BY status;'"
```

## Out of scope (backlog)
A read-only `entertheconstruct stats` subcommand and any HTTP/monitoring server are
explicitly deferred. This document is the contract a separate app would build against.
