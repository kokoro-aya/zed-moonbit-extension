# Cycle 3 Iteration 0 project fixtures

The child modules in this directory are independently complete. The parent
workspace is introduced separately so direct-project and parent-root behavior
can be compared without changing their source authority.

Run the shared authority from this directory:

```sh
moon fmt --check
moon check --deny-warn
moon test
```

`moon.work` lists both modules explicitly. It does not make either module a
dependency of the other.
