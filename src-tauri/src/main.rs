mod ax;
mod dump;

fn main() {
    // Настоящее приложение собирается в задаче 13. Пока — точка входа, чтобы
    // крейт компилировался и `cargo check` ловил ошибки в слое Accessibility.
    println!("accessibility trusted: {}", ax::trusted());
}
