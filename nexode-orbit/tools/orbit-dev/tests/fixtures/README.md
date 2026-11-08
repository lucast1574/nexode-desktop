runtime_memory_reference.json freezes synthetic input and expected JSON from analyze_runtime_memory_log.py at bc26f12. It covers all six incident causes, allocator coverage, client and server hints, process boundaries, missing data, zero denominators and top limits. Only suggested analyzer command paths were changed to the native entrypoint. The reference interpreter is not needed to run the Rust tests.

`reload_audit_reference.json` contains synthetic log/trace/durable/transcript
inputs and the expected report captured from `scripts/reload_recovery_audit.py`
at commit `0574347`, before its removal. The corpus includes eleven reload intents
and preserves the original event ordering and verdict precedence. The Rust test
writes these inputs into a temporary home and compares the report using an empty
PATH; the old implementation is not invoked by tests.
