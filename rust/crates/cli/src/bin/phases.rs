//! Time the phases of a check over files listed in a file (one path per
//! line): lexing, parsing, compile checks and the whole check. A
//! development tool for performance work.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Counts allocations, to find allocation-heavy phases.
struct Counting;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { mimalloc::MiMalloc.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { mimalloc::MiMalloc.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn main() {
    let list = std::env::args().nth(1).expect("usage: phases LIST_FILE");
    let paths: Vec<String> = std::fs::read_to_string(list).unwrap_or_default().lines().map(String::from).collect();
    let sources: Vec<String> = paths.iter().filter_map(|path| std::fs::read_to_string(path).ok()).collect();
    let megabytes = sources.iter().map(String::len).sum::<usize>() as f64 / 1e6;
    let settings = refactrail_engine::Settings {
        profile: "strict".into(),
        line_length: 79,
        function_preferred_lines: 40,
        function_max_lines: 50,
        main_max_lines: 100,
        select: vec!["RT".into()],
        ignore: vec![],
    };
    let only = std::env::var("PHASE").ok();
    let time = |label: &str, run: &dyn Fn(&str)| {
        if only.as_deref().is_some_and(|wanted| wanted != label) {
            return;
        }
        let mut best = f64::MAX;
        let (mut allocations, mut bytes) = (0, 0);
        for _ in 0..(if only.is_some() { 20 } else { 5 }) {
            let (before, before_bytes) = (ALLOCATIONS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed));
            let started = Instant::now();
            for source in &sources {
                run(source);
            }
            best = best.min(started.elapsed().as_secs_f64());
            allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
            bytes = BYTES.load(Ordering::Relaxed) - before_bytes;
        }
        println!(
            "{label:16} {:7.1} ms  {:6.1} MB/s  {:9} allocations  {:7.1} MB allocated",
            best * 1e3,
            megabytes / best,
            allocations,
            bytes as f64 / 1e6
        );
    };
    time("lex", &|source| {
        let _ = refactrail_lexer::tokenize_partial(source, true);
    });
    time("prepare", &|source| {
        let _ = refactrail_parser::prepare_only(source);
    });
    time("parse", &|source| {
        let _ = refactrail_parser::parse_with_comments(source);
    });
    time("parse+compile", &|source| {
        if let Ok((tree, _)) = refactrail_parser::parse_with_comments(source) {
            let _ = refactrail_parser::compile::check(&tree);
        }
    });
    time("full check", &|source| {
        let _ = refactrail_engine::check_file("x.py", source.as_bytes(), &settings);
    });
    refactrail_engine::start_rule_timing();
    for source in &sources {
        let _ = refactrail_engine::check_file("x.py", source.as_bytes(), &settings);
    }
    for (label, seconds) in refactrail_engine::rule_timings() {
        println!("  rule {label:24} {:6.1} ms", seconds * 1e3);
    }
}
