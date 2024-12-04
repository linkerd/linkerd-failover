use std::fmt::{Display, Write};

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
        let width = rows
            .iter()
            .map(|t| (self.value)(t).len())
            .max()
            .unwrap_or(0);
        self.header.len().max(width) + PADDING
    }
}

impl<T> Display for Table<'_, T> {
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
        f.write_char('\n')?;
        // Print data
        for t in self.data {
            for (col, width) in self.cols.iter().zip(&column_widths) {
                let value = (col.value)(t);
                write!(f, "{:width$}", value, width = width)?;
            }
            f.write_char('\n')?;
        }
        Ok(())
    }
}
