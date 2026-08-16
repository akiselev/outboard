#!/usr/bin/env bash
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)";cd "$repo"
root="$(mktemp -d)";trap 'rm -rf "$root"' EXIT
export XDG_CACHE_HOME="$root/cache";mkdir -p "$root/plugin-a" "$root/plugin-b/bin" "$root/host"
echo '== toolchain ==';rustc --version;cargo --version
echo '== static/unit/integration ==';cargo fmt --all;cargo fmt --all -- --check;cargo check --workspace --all-targets;cargo test --workspace --all-targets;cargo clippy --workspace --all-targets
echo '== install external binaries ==';cargo install --path examples/demo-engine-echo --root "$root/plugin-a" --force;cargo install --path examples/demo-host --root "$root/host" --force
host="$root/host/bin/outboard-demo";plugin="$root/plugin-a/bin/outboard-demo-engine-echo";test -x "$host";test -x "$plugin"
echo '== direct control ==';manifest="$($plugin __outboard manifest)";python3 - "$manifest" <<'PY'
import json,sys;m=json.loads(sys.argv[1]);assert m['plugin']['id']=={'namespace':'outboard-demo','kind':'engine','name':'echo'};assert {'one_shot','worker'}<=set(m['execution']);assert any(i['id']=='demo.echo' and i['version']=='1.0.0' for i in m['interfaces']);print('manifest verified')
PY
$plugin __outboard doctor >/dev/null;schema="$($plugin __outboard cli-schema)";python3 - "$schema" <<'PY'
import json,sys;s=json.loads(sys.argv[1]);assert {'echo','delay'}<={x['name'] for x in s['subcommands']};print('CLI schema verified')
PY
$plugin __outboard ping >/dev/null;if $plugin __outboard not-real >/dev/null 2>&1;then echo 'unknown control command succeeded' >&2;exit 1;fi
echo '== PATH discovery ==';PATH="$root/host/bin:$root/plugin-a/bin:$PATH" "$host" plugins list --kind engine --inspect | tee "$root/path.txt";grep -F "$plugin" "$root/path.txt" >/dev/null;PATH="$root/host/bin:$root/plugin-a/bin:$PATH" "$host" plugins doctor engine:echo --deep >/dev/null
echo '== env discovery ==';PATH="$root/host/bin:/usr/bin:/bin" OUTBOARD_DEMO_PLUGIN_PATH="$root/plugin-a/bin" "$host" plugins list --kind engine --inspect | tee "$root/env.txt";grep -F "$plugin" "$root/env.txt" >/dev/null
echo '== explicit no-PATH ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" plugins list --kind engine --inspect >/dev/null
echo '== shadowing ==';cp "$plugin" "$root/plugin-b/bin/outboard-demo-engine-echo";chmod +x "$root/plugin-b/bin/outboard-demo-engine-echo";"$host" --no-path --plugin-dir "$root/plugin-a/bin" --plugin-dir "$root/plugin-b/bin" plugins list --kind engine --inspect --all | tee "$root/shadow.txt";grep -F "$root/plugin-a/bin/outboard-demo-engine-echo" "$root/shadow.txt" >/dev/null;grep -F "$root/plugin-b/bin/outboard-demo-engine-echo" "$root/shadow.txt" >/dev/null;grep '^\* outboard-demo/engine/echo' "$root/shadow.txt" >/dev/null
echo '== explicit override ==';"$host" --no-path --plugin-dir "$root/plugin-b/bin" --echo-override "$plugin" plugins inspect engine:echo | tee "$root/override.txt";grep -F "$plugin" "$root/override.txt" >/dev/null
echo '== typed one-shot ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" echo 'Hello World' --repeat 2 --case upper --tag alpha --tag beta --prefix '>' >"$root/echo.json" 2>"$root/echo.err";cat "$root/echo.err";cat "$root/echo.json";python3 - "$root/echo.json" <<'PY'
import json,sys;v=json.load(open(sys.argv[1]));assert v['text']=='>HELLO WORLD HELLO WORLD';assert v['tags']==['alpha','beta'];assert isinstance(v['process_id'],int);print('one-shot verified')
PY
echo '== worker events ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" worker-echo Worker --case upper --tag persistent | tee "$root/events.txt";grep -F '"type":"started"' "$root/events.txt" >/dev/null;grep -F '"type":"progress"' "$root/events.txt" >/dev/null;grep -F '"type":"output"' "$root/events.txt" >/dev/null;grep -F '"type":"finished"' "$root/events.txt" >/dev/null
echo '== concurrent reuse ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" worker-batch | tee "$root/batch.txt";grep -F 'worker reuse verified: both requests ran in pid' "$root/batch.txt" >/dev/null
echo '== cancellation ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" worker-cancel | tee "$root/cancel.txt";grep -F 'cancellation verified:' "$root/cancel.txt" >/dev/null
echo '== conformance ==';"$host" conformance "$plugin" >"$root/conformance.json";python3 - "$root/conformance.json" <<'PY'
import json,sys;r=json.load(open(sys.argv[1]));bad=[c for c in r['checks'] if c['status']=='fail'];assert not bad,bad;print('conformance checks',len(r['checks']))
PY
if "$host" conformance /bin/echo >/dev/null 2>&1;then echo '/bin/echo passed conformance' >&2;exit 1;fi
echo '== independent protocol probe ==';python3 scripts/protocol-probe.py "$plugin"
echo '== cache ==';"$host" --no-path --plugin-dir "$root/plugin-a/bin" plugins inspect engine:echo >/dev/null;cache="$($host plugins cache path)";test -f "$cache";"$host" plugins cache clear >/dev/null;"$host" --no-path --plugin-dir "$root/plugin-a/bin" plugins inspect engine:echo >/dev/null;test -f "$cache"
echo '== docs ==';cargo doc --workspace --no-deps
echo 'OUTBOARD E2E PASS'
