# geometry-library

An independently runnable, library-shaped MoonBit module. It exposes a small
geometry operation and a project-local `common` package whose names will also
exist in the application fixture.

Run from this directory:

```sh
moon fmt --check
moon check --deny-warn
moon test
moon run cmd/main
```

The final command prints `geometry-library:42`.
