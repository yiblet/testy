use clap::{App, Arg};

pub struct Cli {
    pub no_scroll: bool,
    pub shell: String,
    pub scroll_speed: u8,
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
            .arg(
                Arg::with_name("shell")
                    .long("shell")
                    .value_name("SHELL")
                    .help("bash command to use")
                    .default_value("bash")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("scroll-speed")
                    .long("scroll-speed")
                    .value_name("SPEED")
                    .help("speed of scrolling")
                    .default_value("3")
                    .takes_value(true),
            )
            .get_matches();
        Ok(Cli {
            no_scroll: matches.is_present("no-scroll"),
            shell: matches.value_of("shell").unwrap().to_string(),
            scroll_speed: matches.value_of("scroll-speed").unwrap().parse().unwrap(),
        })
    }
}
