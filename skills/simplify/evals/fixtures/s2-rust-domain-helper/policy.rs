fn parse_customer_policy(value: &str) -> Result<&str, &'static str> {
    match value {
        "retail" => Ok("retail"),
        "wholesale" => Ok("wholesale"),
        _ => Err("unknown customer policy"),
    }
}

fn load_policy(value: &str) -> String {
    match parse_customer_policy(value) {
        Ok(policy) => format!("policy={policy}"),
        Err(error) => format!("error={error}"),
    }
}

fn main() {
    println!("{}", load_policy("retail"));
    println!("{}", load_policy("invalid"));
}
