use std::{fmt::Write as _, ops::Range};

pub(crate) const DIMENSIONS_HELP: &str = "De 1 a 20 columnas y de 1 a 100 filas";

pub(crate) enum SlashCommand {
    Other,
    Invalid,
    Table(Size),
}

pub(crate) struct GeneratedTable {
    pub(crate) markdown: String,
    pub(crate) first_heading: Range<usize>,
}

pub(crate) struct Size {
    columns: u8,
    rows: u8,
}

impl Size {
    fn new(columns: &str, rows: &str) -> Option<Self> {
        let columns = columns.trim().parse().ok()?;
        let rows = rows.trim().parse().ok()?;
        ((1..=20).contains(&columns) && (1..=100).contains(&rows)).then_some(Self { columns, rows })
    }

    pub(crate) fn generate(&self) -> GeneratedTable {
        let mut markdown = String::from("|");
        let mut first_heading = 0..0;
        for column in 1..=self.columns {
            markdown.push(' ');
            let start = markdown.len();
            let _ = write!(markdown, "Columna {column}");
            if column == 1 {
                first_heading = start..markdown.len();
            }
            markdown.push_str(" |");
        }
        markdown.push_str("\n|");
        markdown.push_str(&" --- |".repeat(usize::from(self.columns)));
        let body_row = format!("\n|{}", "   |".repeat(usize::from(self.columns)));
        markdown.push_str(&body_row.repeat(usize::from(self.rows)));
        GeneratedTable {
            markdown,
            first_heading,
        }
    }
}

pub(crate) fn parse_slash(line: &str) -> SlashCommand {
    let Some(dimensions) = line.trim_end().strip_prefix("/table{") else {
        return SlashCommand::Other;
    };
    dimensions
        .strip_suffix('}')
        .and_then(|dimensions| dimensions.split_once("}{"))
        .and_then(|(columns, rows)| Size::new(columns, rows))
        .map_or(SlashCommand::Invalid, SlashCommand::Table)
}
