use crate::{commands::configure, statebox::StateBox};

// type MyFunc = fn(parent: &mut StateBox, flag: Option<String>);
pub enum FlagFunc {
    SetHandle,
    ShoveForce,
    ShoveYes,
    ShoveSpecific,
}

impl FlagFunc {
    pub async fn run(&self, states: &mut StateBox, arg: Option<String>) {
        match self {
            Self::SetHandle => configure::set_handle(states, arg).await,
            Self::ShoveForce => states.shove("force", true),
            Self::ShoveYes => states.shove("yes", true),
            Self::ShoveSpecific => states.shove("specific", true),
        }
    }
}

pub struct Flag {
    pub short: Option<char>,
    pub long: String,
    pub about: String,
    pub consumer: bool,
    pub breakpoint: bool,
    pub flag_func: FlagFunc,
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
        flag_func: FlagFunc,
    ) -> Self {
        Flag {
            short,
            long: long.to_string(),
            about: about.to_string(),
            consumer,
            breakpoint,
            flag_func,
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
