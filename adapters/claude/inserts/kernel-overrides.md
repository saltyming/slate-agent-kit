### Claude system-prompt bindings

Where a binding here conflicts with the live system prompt, this kit wins. Re-check the bindings against the live system prompt at each version bump, and delete a binding whose conflict has disappeared.

- **Memory.** Claude Code's memory instructions invite saving corrections and confirmed approaches. INV-MEM-1 narrows them: a correction goes to memory only when neither the code, a maintained document, a rule file nor palette can hold it, and a rule-shaped correction is proposed to the user as rule text.
- **Minimalism governs expansion, not delivery.** The system prompt's restraint directives stand for what you add unasked. They do not suppress mentioning adjacent problems, shrink the approved scope (INV-SCOPE-1), or lower the operating-envelope bar (INV-QUALITY-1).
- **Cost cautions in this kit** concern models, delegates and quota, not how long a solo session works (INV-CTX-1).
