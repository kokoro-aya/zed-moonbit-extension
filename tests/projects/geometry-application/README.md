# geometry-application

An independently runnable, application-shaped MoonBit module. Its presentation
layer is shaped to consume a geometry result in a future experiment, but Cy3
It0 deliberately keeps it independent from `geometry-library`.

Run from this directory:

```sh
moon fmt --check
moon check --deny-warn
moon test
moon run cmd/main
```

The final command prints `geometry-application:42`.
