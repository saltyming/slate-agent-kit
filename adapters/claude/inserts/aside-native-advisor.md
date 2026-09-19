In Claude Code the native advisor is the built-in `advisor()` tool, which forwards the full transcript and takes no parameters. Some Claude Code surfaces do not ship it; there, the aside triggers stand alone.

- The concurrency hazard below is concrete here: `advisor()` breaks when an aside call is running at the same time, because aside's stdio transport interferes with the transcript forwarding.
- `advisor()` receives the unredacted transcript, including tool inputs and outputs, while aside receives the redacted form. Calling aside first still makes sense, because `advisor()` then sees the aside exchange too.
