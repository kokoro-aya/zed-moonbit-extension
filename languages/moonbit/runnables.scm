(structure
  (function_definition
    (function_identifier
      (lowercase_identifier) @run))
  (#eq? @run "main")
  (#set! tag "moon-run"))

(structure
  (test_definition
    "test" @run)
  (#set! tag "moon-test"))
