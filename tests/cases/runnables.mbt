///|
fn helper() -> Int {
  42
}

///|
fn outer() -> Int {
  fn main() -> Int {
    41
  }
  main() + 1
}

///|
fn main {
  println(helper())
}

///|
test "top-level test" {
  inspect(outer(), content="42")
}

///|
test "second top-level test" {
  inspect(helper(), content="42")
}
