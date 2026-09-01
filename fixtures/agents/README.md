# Agent test fixtures

Everything in this directory is synthetic test input. In particular,
`secret-corpus.json` contains deliberately fake credentials used to prove that
redaction and secret-persistence checks fail when they should. None of these
values grants access to a real service, account, machine, or network.

Keep the corpus committed: it is security test data, not production evidence.
Do not replace its values with real credentials.
