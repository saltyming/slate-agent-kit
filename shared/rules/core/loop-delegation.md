<!-- slate-agent-kit:common -->
# Work Leaving the Session

Subagents, consultation (`{{ASIDE_RULE_FILE}}`) and dispatch (`{{DISPATCH_RULE_FILE}}`), each judged for value (§ 7) and used at its level (§ 8).

- Subagents are the harness's delegates: read-only ones inspect, search or summarize; write-capable ones edit; an unknown kind is write-capable. Consultation asks for an opinion and writes nothing. Dispatch hands a self-contained, write-capable step to an external agent that runs asynchronously. Skills, commands and workflows bypass no article. When the harness lacks a mechanism safe delegation needs, say so; do not improvise one.
- A delegate saves the leader's context and costs the user models, quota and time, returning a result without its reasoning. It helps when work splits into independent parts with a stable shared contract, or when one bounded lookup would flood the context. Work stays in-session when sequential, tightly coupled, small, assigned to the leader by a document, or when the user is waiting for the leader's own answer. At `auto`, state in one line before starting how many delegates, which model, what each does and which files each writes; at `suggest`, say the same and wait. A read-only delegate does not need the session's top model.
- Write the shared contracts (public types, schemas, migration order, shared tests) before delegating; one writer per file (§ 14). One prompt over many inputs needs a tight template. A prompt is self-contained: files owned, expected output, what success looks like, settled decisions as constraints, what must not be done, the operating envelope (§ 9). A delegate cannot watch a long-running process; have it write a log, a results file or an exit-code file. Implementation work never goes to a read-only delegate.

{{@INSERT delegation-surfaces}}
