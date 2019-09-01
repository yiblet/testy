use clap::{App, Arg};

pub struct Cli {
    pub no_scroll: bool,
}

impl Cli {
    pub fn parse() -> Result<Cli, String> {
        let matches = App::new("testy")
            .version("0.1")
            .author("Shalom Yiblet <shalom.yiblet@gmail.com>")
            .about("test your pipes")
            .arg(
                Arg::with_name("no-scroll")
                    .short("s")
                    .long("no-scroll")
                    .value_name("SCROLL")
                    .help("do not scroll")
                    .takes_value(false),
            )
            .get_matches();
        Ok(Cli {
            no_scroll: matches.is_present("no-scroll"),
        })
    }
}
