(structure
  (function_definition
    (function_identifier
      (lowercase_identifier) @run))
  (#eq? @run "main")
  (#set! tag "moon-run"))

(structure
  (test_definition
    "test" @run
    (string_literal
      (string_fragment
        (unescaped_string_fragment) @MOONBIT_TEST_NAME)))
  (#set! tag "moon-test"))
