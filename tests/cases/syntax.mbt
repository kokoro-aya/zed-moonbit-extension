///| A deliberately varied source used by the pinned grammar audit.
pub struct Point {
  x : Int
  y : Int
}

///|
pub enum Shape {
  Dot(Point)
  Segment(Point, Point)
}

///|
pub fn measure(shape : Shape) -> Int {
  match shape {
    Dot(point) => point.x + point.y
    Segment(start, end) => end.x - start.x
  }
}

///|
test "measure a point" {
  inspect(measure(Dot({ x: 20, y: 22 })), content="42")
}
