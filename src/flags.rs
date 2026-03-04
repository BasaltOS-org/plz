use tokio::runtime::Runtime;

use crate::statebox::StateBox;

type MyFunc = fn(rt: &Runtime, parent: &mut StateBox, flag: Option<String>);

pub struct Flag {
    pub short: Option<char>,
    pub long: String,
    pub about: String,
    pub consumer: bool,
    pub breakpoint: bool,
    pub run_func: MyFunc,
}

impl PartialEq for Flag {
    // Superfluous PartialEq implementation to allow for struct field equality checks.
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Flag {
    pub fn new(
        short: Option<char>,
        long: &str,
        about: &str,
        consumer: bool,
        breakpoint: bool,
        run_func: MyFunc,
    ) -> Self {
        Flag {
            short,
            long: long.to_string(),
            about: about.to_string(),
            consumer,
            breakpoint,
            run_func,
        }
    }
    pub fn help(&self) -> String {
        let mut help = String::new();
        let short = if let Some(short) = self.short {
            format!("-{short},")
        } else {
            String::from("   ")
        };
        help.push_str(&format!("{} --{}\t{}", short, self.long, self.about));
        help
    }
}
