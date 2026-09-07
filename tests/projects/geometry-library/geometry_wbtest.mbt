///|
test "scale a measurement" {
  inspect(scale(21), content="42")
}

///|
test "scale another measurement" {
  inspect(scale(10), content="20")
}
