# Ralph Wiggum Dogfood Loop

This dogfood fixture is intentionally tiny. The "project" is a single text file
that a worker appends to three times. The point is not the output; the point is
to feel the Armature authoring and harness loop as a user.

## Simplest Loop

[`ralph-simplest.armature`](ralph-simplest.armature) is the smallest useful
version: no coerce, no data, no profile policy, no `maxActive`, no internal
helper event. It just starts `ralph`, waits for `finished`, starts the next
line, and stops after the third completion.

```sh
store=dogfood/ralph-wiggum/.armature/ralph-simplest.sqlite

rm -f "$store" dogfood/ralph-wiggum/project/ralph-output.txt

cargo run -p armature-cli -- validate dogfood/ralph-wiggum/ralph-simplest.armature --json

cargo run -p armature-cli -- run dogfood/ralph-wiggum/ralph-simplest.armature \
  --store "$store" \
  --event begin \
  --payload '{"message":"go"}' \
  --json

cargo run -p armature-cli -- harness run dogfood/ralph-wiggum/ralph-simplest.armature \
  --store "$store" \
  --config dogfood/ralph-wiggum/harness.json \
  --drive-workflow \
  --max-iterations 10 \
  --json

cargo run -p armature-cli -- status dogfood/ralph-wiggum/ralph-simplest.armature \
  --store "$store" \
  --compact
```

Expected output:

```text
Ralph: I made a folder.
Ralph: I wrote the important sentence.
Ralph: I checked my work.
```

## Sequential MaxActive Loop

One important bit of shape: after each `finished` event, the workflow raises a
small internal `next` event before starting the following worker. That keeps a
`maxActive 1` worker truly sequential because active-invocation retirement is
visible after the completion event is processed.

Run [`ralph-wiggum-loop.armature`](ralph-wiggum-loop.armature) from the
repository root:

```sh
store=dogfood/ralph-wiggum/.armature/ralph.sqlite

rm -f "$store" dogfood/ralph-wiggum/project/ralph-output.txt

cargo run -p armature-cli -- validate dogfood/ralph-wiggum/ralph-wiggum-loop.armature --json

cargo run -p armature-cli -- run dogfood/ralph-wiggum/ralph-wiggum-loop.armature \
  --store "$store" \
  --event begin \
  --payload '{"project":"project/ralph-output.txt"}' \
  --json

cargo run -p armature-cli -- harness run dogfood/ralph-wiggum/ralph-wiggum-loop.armature \
  --store "$store" \
  --config dogfood/ralph-wiggum/harness.json \
  --drive-workflow \
  --max-iterations 10 \
  --json

cargo run -p armature-cli -- status dogfood/ralph-wiggum/ralph-wiggum-loop.armature \
  --store "$store" \
  --compact
```

Expected project output:

```text
Ralph: I made a folder.
Ralph: I wrote the important sentence.
Ralph: I checked my work and it is cromulent.
```
