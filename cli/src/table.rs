use std::fmt::Display;

const PADDING: usize = 3;
pub struct Column<T> {
    header: &'static str,
    value: Box<dyn Fn(&T) -> String>,
}

pub struct Table<'a, T> {
    pub cols: Vec<Column<T>>,
    pub data: &'a [T],
}

impl<T> Column<T> {
    pub fn new(header: &'static str, value: Box<dyn Fn(&T) -> String>) -> Column<T> {
        Column { header, value }
    }

    fn width(&self, rows: &[T]) -> usize {
        match rows.iter().map(|t| (self.value)(t).len()).max() {
            Some(width) => self.header.len().max(width) + PADDING,
            None => self.header.len() + PADDING,
        }
    }
}

impl<'a, T> Display for Table<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let column_widths = self
            .cols
            .iter()
            .map(|col| col.width(self.data))
            .collect::<Vec<usize>>();
        // Print headers
        for (col, width) in self.cols.iter().zip(&column_widths) {
            write!(f, "{:width$}", col.header, width = width)?;
        }
        writeln!(f)?;
        // Print data
        for t in self.data {
            for (col, width) in self.cols.iter().zip(&column_widths) {
                let value = (col.value)(t);
                write!(f, "{:width$}", value, width = width)?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
