//! The fake gateway as a process, for scripts (`script/budget.sh`): prints its URL, answers
//! every prompt with a short text, and runs until killed.

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Reply};

fn main() {
    let gateway = FakeGateway::start(|request, _| {
        let model = request.json()["model"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        Reply::Sse {
            body: Stream::new().text("Done.").finish(&model, Some(1)),
            before: Some(731),
        }
    });
    println!("{}", gateway.url);
    loop {
        std::thread::park();
    }
}
