#[test]
fn dbg_details_dump() {
    use ratatui::backend::TestBackend;
    let backend = TestBackend::new(120, 40);
    let mut t = ratatui::Terminal::new(backend).unwrap();
    let mut app = App::new();
    app.history.clear();
    app.view = View::Details;
    t.draw(|f| draw(f, &app)).unwrap();
    let buf = t.backend().buffer().clone();
    for y in 0..22 {
        let row: String = (0..110).map(|x| buf[(x, y)].symbol()).collect();
        println!("{y:>2}|{row}");
    }
}
