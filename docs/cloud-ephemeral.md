# Ephemeral Nexode Cloud machines

This is the infrastructure foundation for isolated benchmarks and other short-lived
remote jobs. It does not implement a benchmark dashboard or automatically run a
model. The backend implementation lives in the separate
`solosystems-managed-cloud` repository.

## Isolation and lifecycle

`nexode cloud machine` still manages the account's persistent general-purpose host.
`nexode cloud ephemeral` addresses a separate machine collection. It never uses
`cloud move` or copies a live local session, its repository, or provider credentials.
Each machine has its own SSH identity and pinned host-key file under
`~/.nexode/cloud/ephemeral/<machine-id>/`. Native SSH attachment renders the actual
Nexode TUI locally while the server and tools run remotely. No tmux is needed.

Every create request has a hard lifetime of 300 through 86,400 seconds, defaulting
to one hour. The lifetime includes provisioning, bootstrap and disconnected time.
Closing the TUI does not end the job or extend its deadline. The backend is
responsible for TTL enforcement, automatic deletion, resource limits, ownership
checks, VM credential revocation and cloud resource cleanup. **All files on the
machine are disposable. Export results before expiry or termination.**

The initial backend uses an operator-controlled fixed CPU machine size and caps
concurrent machines. Clients cannot request arbitrary GPU/instance sizes or
unbounded lifetimes. These machines use Nexode Cloud resources, not Modal credits.
Model requests remain subject to the chosen provider's billing and limits.

## Deployment prerequisite

The CLI requires the new `/v1/cloud/machines` API, its additive database migration,
and the backend's ephemeral feature flag. CLI compilation or mocked tests do not
prove the API is deployed. Follow `docs/CLOUD_MACHINES.md` in the backend repo and
validate a short-lived canary before enabling it broadly.

Keep the persistent machine stopped/unchanged during the canary. Record its state
before and after. Do not claim production acceptance until a real machine has
been created, reached SSH, run preflight commands, exported its results, and been
verified terminated, including the automatic-expiry case.

## CLI

```sh
# Create returns promptly. Provisioning continues separately.
nexode cloud ephemeral create --name bench-pilot --ttl-seconds 1800 --json

# Inspect without waking the persistent machine.
nexode cloud ephemeral list --json
nexode cloud ephemeral status cm_0123456789abcdef0123456789abcdef --json

# Execute commands, preserving stdin and stdout for upload/download pipelines.
nexode cloud ephemeral exec cm_0123456789abcdef0123456789abcdef -- uname -m

# Attach the real Nexode TUI. This waits for the new machine's SSH readiness.
nexode cloud ephemeral attach cm_0123456789abcdef0123456789abcdef

# Irreversible: destroys this ephemeral machine and its disk.
nexode cloud ephemeral terminate cm_0123456789abcdef0123456789abcdef --yes --json
```

Replace the example ID with the ID returned by create. A create request prints its
UUID to stderr *before* sending it. If the network fails, retry with the same
`--request-id UUID` and identical settings to avoid creating a second machine.
JSON results go to stdout. The request ID and progress go to stderr.

## Pinning a benchmark configuration

A machine's bootstrap Nexode version is not necessarily the version being tested.
Do not update a shared daemon or assume the bootstrap build supports the current
native SSH protocol. Upload the exact candidate binary and use a dedicated daemon
socket. The following is a manual setup recipe, not an automatic benchmark run:

```sh
ID=cm_0123456789abcdef0123456789abcdef
BIN=/absolute/path/to/the/linux-x86_64/nexode

# Record this locally with the benchmark revision, compiler, task and model config.
sha256sum "$BIN"
nexode cloud ephemeral exec "$ID" -- bash -lc 'mkdir -p "$HOME/bench"'
nexode cloud ephemeral exec "$ID" -- bash -lc \
  'cat > "$HOME/bench/nexode.tmp" && chmod 700 "$HOME/bench/nexode.tmp" && mv "$HOME/bench/nexode.tmp" "$HOME/bench/nexode"' < "$BIN"
nexode cloud ephemeral exec "$ID" -- bash -lc \
  'sha256sum "$HOME/bench/nexode"; "$HOME/bench/nexode" --version; cc --version; valgrind --version'

# Upload a pristine benchmark checkout, without local credentials/build products.
git -C /path/to/nexode-bench archive HEAD | \
  nexode cloud ephemeral exec "$ID" -- bash -lc \
  'mkdir -p "$HOME/bench/tasks-src" && tar xf - -C "$HOME/bench/tasks-src"'
```

Before launching, compare the remote binary hash to the local hash. Check the
candidate's `server stdio` protocol and shared-library compatibility. Record the
resolved provider, model, reasoning settings, tool policy and concurrency. The
SSH attachment deliberately does not override an already-running daemon's model
or tool configuration: set those when starting the dedicated remote daemon.

For example, with a supported candidate and a deliberately selected model:

```sh
nexode cloud ephemeral exec "$ID" -- bash -lc \
  'nohup "$HOME/bench/nexode" --no-update --no-selfdev --provider nexode --model MODEL --socket /run/user/1000/bench.sock serve > "$HOME/bench/server.log" 2>&1 < /dev/null &'

nexode cloud ephemeral attach "$ID" \
  --remote-binary /home/ec2-user/bench/nexode \
  --server-socket /run/user/1000/bench.sock \
  --workdir /home/ec2-user/bench/tasks-src/tasks/json-unescape
```

Replace `MODEL` with the intended provider-supported model. Starting model work
can consume provider credit. The dedicated socket prevents the bootstrap daemon
from silently serving a different Nexode binary/configuration. Use one machine
per run initially; separate machines per task provide stronger resource isolation
when running tasks in parallel.

Retrieve artifacts while the machine is alive, then terminate it explicitly:

```sh
nexode cloud ephemeral exec "$ID" -- bash -lc \
  'tar czf - -C "$HOME/bench" tasks-src server.log' > bench-artifacts.tar.gz
nexode cloud ephemeral terminate "$ID" --yes
```

Do not include provider credentials in exported archives. A later benchmark runner
should automate manifests, score updates, artifact retrieval and cleanup while
retaining these same lifecycle and isolation guarantees.
