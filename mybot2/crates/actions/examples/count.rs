fn main() {
    let all = mybot_actions::all();
    println!("{} actions", all.len());
    for (c, n) in mybot_actions::categories() { println!("  {c}: {n}"); }
    let examples = all.iter().filter(|a| a.example.is_some()).count();
    println!("{examples} with runnable examples");
}
