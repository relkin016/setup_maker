//! Boolean condition expressions in Inno style:
//! `text\rus or voice\rus`, `not Is64BitInstallMode`, `a and (b or c)`,
//! `FileExists('{app}\x.exe')`. Adjacent operands without an operator are
//! OR-ed (Inno `Components: a b`).

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ident(String),
    Call(String, Vec<String>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Const(bool),
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    LParen,
    RParen,
    Comma,
    And,
    Or,
    Not,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if ch == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if ch == '!' {
            out.push(Tok::Not);
            i += 1;
        } else if ch == '\'' || ch == '"' {
            let q = ch;
            i += 1;
            let mut v = String::new();
            loop {
                if i >= c.len() {
                    return Err("unterminated string".into());
                }
                if c[i] == q {
                    if i + 1 < c.len() && c[i + 1] == q {
                        v.push(q);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                v.push(c[i]);
                i += 1;
            }
            out.push(Tok::Str(v));
        } else {
            let mut v = String::new();
            while i < c.len() && !c[i].is_whitespace() && !"(),'\"!".contains(c[i]) {
                v.push(c[i]);
                i += 1;
            }
            match v.to_ascii_lowercase().as_str() {
                "and" => out.push(Tok::And),
                "or" => out.push(Tok::Or),
                "not" => out.push(Tok::Not),
                _ => out.push(Tok::Ident(v)),
            }
        }
    }
    Ok(out)
}

struct P {
    t: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn or(&mut self) -> Result<Expr, String> {
        let mut l = self.and()?;
        loop {
            match self.peek() {
                Some(Tok::Or) => {
                    self.i += 1;
                    let r = self.and()?;
                    l = Expr::Or(Box::new(l), Box::new(r));
                }
                // implicit OR between adjacent operands
                Some(Tok::Ident(_)) | Some(Tok::Not) | Some(Tok::LParen) => {
                    let r = self.and()?;
                    l = Expr::Or(Box::new(l), Box::new(r));
                }
                _ => return Ok(l),
            }
        }
    }
    fn and(&mut self) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while let Some(Tok::And) = self.peek() {
            self.i += 1;
            let r = self.unary()?;
            l = Expr::And(Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Tok::Not) => {
                self.i += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            Some(Tok::LParen) => {
                self.i += 1;
                let e = self.or()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err("expected ')'".into());
                }
                self.i += 1;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                self.i += 1;
                if self.peek() == Some(&Tok::LParen) {
                    self.i += 1;
                    let mut args = Vec::new();
                    loop {
                        match self.peek().cloned() {
                            Some(Tok::RParen) => {
                                self.i += 1;
                                break;
                            }
                            Some(Tok::Str(s)) | Some(Tok::Ident(s)) => {
                                args.push(s);
                                self.i += 1;
                            }
                            Some(Tok::Comma) => self.i += 1,
                            _ => return Err(format!("bad arguments for {}", name)),
                        }
                    }
                    Ok(Expr::Call(name, args))
                } else if name.eq_ignore_ascii_case("true") {
                    Ok(Expr::Const(true))
                } else if name.eq_ignore_ascii_case("false") {
                    Ok(Expr::Const(false))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            other => Err(format!("unexpected token {:?}", other)),
        }
    }
}

pub fn parse(s: &str) -> Result<Expr, String> {
    let t = tokenize(s)?;
    if t.is_empty() {
        return Ok(Expr::Const(true));
    }
    let mut p = P { t, i: 0 };
    let e = p.or()?;
    if p.i != p.t.len() {
        return Err(format!("unexpected trailing tokens in '{}'", s));
    }
    Ok(e)
}

impl Expr {
    pub fn eval(&self, f: &mut dyn FnMut(&str, &[String]) -> bool) -> bool {
        match self {
            Expr::Const(b) => *b,
            Expr::Ident(n) => f(n, &[]),
            Expr::Call(n, a) => f(n, a),
            Expr::Not(e) => !e.eval(f),
            Expr::And(a, b) => a.eval(f) && b.eval(f),
            Expr::Or(a, b) => a.eval(f) || b.eval(f),
        }
    }

    pub fn idents(&self, out: &mut Vec<String>) {
        match self {
            Expr::Ident(n) => out.push(n.clone()),
            Expr::Not(e) => e.idents(out),
            Expr::And(a, b) | Expr::Or(a, b) => {
                a.idents(out);
                b.idents(out);
            }
            _ => {}
        }
    }
}

/// Evaluate; empty string is `true`; parse errors evaluate to `true`
/// (validated by the builder beforehand).
pub fn eval_str(s: &str, f: &mut dyn FnMut(&str, &[String]) -> bool) -> bool {
    if s.trim().is_empty() {
        return true;
    }
    match parse(s) {
        Ok(e) => e.eval(f),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        let sel = ["text\\rus", "x"];
        let mut f = |n: &str, _: &[String]| sel.contains(&n);
        assert!(eval_str("text\\rus or voice", &mut f));
        assert!(eval_str("voice text\\rus", &mut f));
        assert!(!eval_str("not x", &mut f));
        assert!(eval_str("x and (voice or text\\rus)", &mut f));
        let mut g = |n: &str, a: &[String]| n == "FileExists" && a[0] == "{app}\\a b.exe";
        assert!(eval_str("FileExists('{app}\\a b.exe')", &mut g));
    }
}
