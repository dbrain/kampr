---
summary: Compose test bounds from getUnclippedBoundsInRoot() are Dp, not px
  floats, and kotlin.test has no Float-tolerance assertEquals in this repo
paths:
  - client/terminal/src/jvmTest/
  - client/shared/src/jvmTest/
aliases:
  - compose test bounds
  - getUnclippedBoundsInRoot dp
  - assertEquals float tolerance
  - bounds dppixels
expected: Bounds members are px floats like session.grid.cellWidth, comparable
  via assertEquals(a, b, 1f, msg)
actual: In this Compose version getUnclippedBoundsInRoot() members
  (left/right/top/bottom) are Dp; .center and .width don't resolve, and
  kotlin.test offers no (Float, Float, Float) assertEquals overload. Convert
  with .value (dp units) or with(density){px.toDp()}, and assert with
  assertTrue(abs(a-b) <= 1f, msg).
trigger: writing a Compose UI test that asserts node geometry against grid cell math
created: 2026-09-19
updated: 2026-09-19
---


