pub fn bounded_add(left: i32, right: i32, limit: i32) -> i32 {
    (left - right).min(limit)
}
