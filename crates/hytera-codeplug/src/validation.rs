//! Which fields refer to other records, from CPS's own configuration files (loaded from the CPS
//! install or the RCDB copy, never bundled). See `docs/file-format.md` ("References").
//!
//! - `DataValidationConfig.config`, `<ParamDependencyItems>`: a parameter (layout id) whose value
//!   points into another block, how the value locates the record (`RowIndex`/`TableIndex` with a
//!   start index, or a parameter value), what is shown for it (a parameter of the source record,
//!   the source table's name, or the row number), special values (`<ExtendValues>`, e.g.
//!   `0 = None`, `65535 = CurrentChanel`) and filters on the source records.
//! - `DataValidationConfig.config`, `<DialRuleItems>`: parameters holding a call ID (contacts).
//! - `ComplexRuleConfig.Config`: virtual parameters (negative ids such as `-7`, a scan list
//!   member) built from an index and a type parameter and named by the referenced record's alias.
//!
//! Parameter ids here are layout ids (`Param.id`, as in `RcdbJsons`), the same ids as
//! [`crate::layout::FieldDef::id`]; block ids are RCDB block ids. Observed for V1.5.24.000.

use std::path::PathBuf;

use crate::rcdb::RcdbRoot;
use crate::{Error, Result};

/// The configuration files read from the RCDB root (CPS install folder).
pub const VALIDATION_FILE: &str = "DataValidationConfig.config";
pub const COMPLEX_RULE_FILE: &str = "ComplexRuleConfig.Config";

/// How a reference value locates the source record (`<OrientateData LinkType=..>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locate {
    /// Position of the record (row) in the source block; `start` is the value of the first row.
    RowIndex { start: i64 },
    /// Position of the table (one member list of a two-level block); `start` as above.
    TableIndex { start: i64 },
    /// The value of parameter `LinkIndex` of the source record.
    ParamValue { link_index: u32 },
    /// Any other kind (not used by hyt).
    Other(String),
}

/// What is shown for the referenced record (`<ShowData ShowType=..>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    /// The source table's name (for two-level blocks: the alias in the companion file).
    TableName,
    /// The value of the source parameter with this `LinkIndex`.
    ParamValue { link_index: u32 },
    /// The row number.
    RowIndex,
    Other(String),
}

/// A filter on the source records (`FilterType="InCollection"`: only records whose parameter has
/// one of `keys`, CPS's UI keys such as `Off`, `Close`, `OPEN`, `GroupCall`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub include: bool,
    pub keys: Vec<String>,
}

/// One `<LinkParam>` of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceParam {
    /// Layout id, or a virtual parameter (negative, see [`VirtualParam`]).
    pub param: i64,
    pub link_index: Option<u32>,
    pub filter: Option<Filter>,
}

/// A field that refers to records of another block (`<ParamDependencyItem>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamReference {
    /// Layout id of the referring field.
    pub param: u32,
    /// The block CPS names for the referring field (for scan lists this is the member block 619
    /// although the field is in the companion block 620), when given.
    pub block: Option<u16>,
    pub locate: Locate,
    pub show: Show,
    /// Special values and CPS's UI key for them (`None`, `CurrentChanel`, `DefaultOption`, …).
    pub special: Vec<(i64, String)>,
    pub source_block: u16,
    pub source_params: Vec<SourceParam>,
}

impl ParamReference {
    /// The source parameter with `LinkIndex` `index`.
    pub fn source_param(&self, index: u32) -> Option<&SourceParam> {
        self.source_params.iter().find(|p| p.link_index == Some(index))
    }

    /// Source parameters that filter the offered records.
    pub fn filters(&self) -> impl Iterator<Item = (i64, &Filter)> {
        self.source_params.iter().filter_map(|p| Some((p.param, p.filter.as_ref()?)))
    }
}

/// A related block of a virtual parameter: records of `block` are meant when the type parameter
/// has `type_value`, named by `alias_param`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedBlock {
    pub block: u16,
    pub type_value: Option<i64>,
    pub alias_param: Option<u32>,
}

/// A virtual parameter from `ComplexRuleConfig.Config` (e.g. `-7`: a scan list member, made of
/// `IndexParamId` 1182 = channel number and `TypeParamId` 1183 = digital/analog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualParam {
    pub id: i64,
    pub block: u16,
    pub index_param: Option<u32>,
    pub type_param: Option<u32>,
    pub related: Vec<RelatedBlock>,
}

/// A parameter holding a call ID (`<DialRuleItems>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialParam {
    pub param: u32,
    pub block: Option<u16>,
    /// Parameter giving the call type, when the type is not fixed.
    pub call_type_param: Option<u32>,
    /// Fixed call type (`Private`, `Group`), when given.
    pub call_type: Option<String>,
    /// `ProductType` of the rule (`Conventional`, `Trunking`).
    pub product: String,
}

/// What CPS's configuration says about references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationConfig {
    pub references: Vec<ParamReference>,
    pub virtual_params: Vec<VirtualParam>,
    pub dial_params: Vec<DialParam>,
}

fn attr<T: std::str::FromStr>(n: &roxmltree::Node, name: &str) -> Option<T> {
    n.attribute(name).and_then(|v| v.trim().parse().ok())
}

fn child<'a, 'i>(n: &roxmltree::Node<'a, 'i>, tag: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(tag))
}

fn parse_doc(xml: &str) -> Result<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml.trim_start_matches('\u{FEFF}')).map_err(|source| Error::Xml { path: PathBuf::new(), source })
}

fn source_params(source: &roxmltree::Node) -> Vec<SourceParam> {
    source
        .descendants()
        .filter(|n| n.has_tag_name("LinkParam"))
        .filter_map(|p| {
            let filter = p.attribute("FilterType").map(|t| Filter {
                include: t != "OutCollection",
                keys: p
                    .descendants()
                    .filter(|k| k.has_tag_name("FilterValueKey"))
                    .filter_map(|k| k.text().map(|t| t.trim().to_string()))
                    .collect(),
            });
            Some(SourceParam { param: attr(&p, "ParamId")?, link_index: attr(&p, "LinkIndex"), filter })
        })
        .collect()
}

/// Parse `DataValidationConfig.config` (references and dial rules).
pub fn parse_data_validation(xml: &str) -> Result<ValidationConfig> {
    let doc = parse_doc(xml)?;
    let mut config = ValidationConfig::default();
    for item in doc.descendants().filter(|n| n.has_tag_name("ParamDependencyItem")) {
        let Some(param) = attr::<u32>(&item, "ParamId") else { continue };
        let locate = match child(&item, "OrientateData") {
            Some(o) => match o.attribute("LinkType").unwrap_or("") {
                "RowIndex" => Locate::RowIndex { start: attr(&o, "StartIndex").unwrap_or(0) },
                "TableIndex" => Locate::TableIndex { start: attr(&o, "StartIndex").unwrap_or(0) },
                "ParamValue" => Locate::ParamValue { link_index: attr(&o, "LinkIndex").unwrap_or(0) },
                other => Locate::Other(other.to_string()),
            },
            None => Locate::Other(String::new()),
        };
        let show = match child(&item, "ShowData") {
            Some(s) => match s.attribute("ShowType").unwrap_or("") {
                "TableName" => Show::TableName,
                "RowIndex" => Show::RowIndex,
                "ParamValue" => Show::ParamValue { link_index: attr(&s, "LinkIndex").unwrap_or(0) },
                other => Show::Other(other.to_string()),
            },
            None => Show::Other(String::new()),
        };
        let special = item
            .descendants()
            .filter(|n| n.has_tag_name("ExtendValue"))
            .filter_map(|e| Some((attr(&e, "Value")?, e.attribute("UIValueKey")?.to_string())))
            .collect::<Vec<_>>();
        for source in item.children().filter(|c| c.has_tag_name("LinkSource")) {
            let Some(source_block) = attr(&source, "BlockId") else { continue };
            config.references.push(ParamReference {
                param,
                block: attr(&item, "BlockId"),
                locate: locate.clone(),
                show: show.clone(),
                special: special.clone(),
                source_block,
                source_params: source_params(&source),
            });
        }
    }
    for rule in doc.descendants().filter(|n| n.has_tag_name("DialRuleItem")) {
        let product = rule.attribute("ProductType").unwrap_or("").to_string();
        for p in rule.descendants().filter(|n| n.has_tag_name("DialParam")) {
            let Some(param) = attr(&p, "ParamId") else { continue };
            config.dial_params.push(DialParam {
                param,
                block: attr(&p, "BlockId"),
                call_type_param: attr(&p, "CallTypeParamId"),
                call_type: p.attribute("CallType").map(str::to_string),
                product: product.clone(),
            });
        }
    }
    Ok(config)
}

/// Parse `ComplexRuleConfig.Config` (virtual parameters).
pub fn parse_complex_rules(xml: &str) -> Result<Vec<VirtualParam>> {
    let doc = parse_doc(xml)?;
    Ok(doc
        .descendants()
        .filter(|n| n.has_tag_name("ComplexRule"))
        .filter_map(|r| {
            Some(VirtualParam {
                id: attr(&r, "VirtualParamId")?,
                block: attr(&r, "BlockID")?,
                index_param: attr(&r, "IndexParamId"),
                type_param: attr(&r, "TypeParamId"),
                related: r
                    .descendants()
                    .filter(|n| n.has_tag_name("RelatedBlock"))
                    .filter_map(|b| {
                        Some(RelatedBlock {
                            block: attr(&b, "BlockID")?,
                            type_value: attr(&b, "TypeValue"),
                            alias_param: attr(&b, "AliasParamId"),
                        })
                    })
                    .collect(),
            })
        })
        .collect())
}

impl ValidationConfig {
    pub fn virtual_param(&self, id: i64) -> Option<&VirtualParam> {
        self.virtual_params.iter().find(|v| v.id == id)
    }
}

impl RcdbRoot {
    /// CPS's reference configuration: `DataValidationConfig.config` (required) and
    /// `ComplexRuleConfig.Config` (optional) in the RCDB root.
    pub fn validation_config(&self) -> Result<ValidationConfig> {
        let read = |name: &str| {
            let path = self.path().join(name);
            std::fs::read_to_string(&path).map_err(|source| Error::Io { path, source })
        };
        let with_path = |name: &str, e: Error| match e {
            Error::Xml { source, .. } => Error::Xml { path: self.path().join(name), source },
            other => other,
        };
        let mut config = parse_data_validation(&read(VALIDATION_FILE)?).map_err(|e| with_path(VALIDATION_FILE, e))?;
        if let Ok(text) = read(COMPLEX_RULE_FILE) {
            config.virtual_params = parse_complex_rules(&text).map_err(|e| with_path(COMPLEX_RULE_FILE, e))?;
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DV: &str = r#"<?xml version="1.0" encoding="utf-8" ?>
<DataValidationConfig>
  <TableMultiParamDependencyItems/>
  <ParamDependencyItems>
    <ParamDependencyItem ParamId="18" BlockId="1">
      <OrientateData LinkType="TableIndex" StartIndex="1"/>
      <ShowData ShowType="TableName"/>
      <ExtendValues><ExtendValue Value="0" UIValueKey="None"/></ExtendValues>
      <LinkSource BlockId="607"/>
    </ParamDependencyItem>
    <ParamDependencyItem ParamId="3412" BlockId="1">
      <OrientateData LinkType="RowIndex" StartIndex="1"/>
      <ShowData ShowType="ParamValue" LinkIndex="0"/>
      <ExtendValues>
        <ExtendValue Value="0" UIValueKey="CurrentChanel"/>
        <ExtendValue Value="65535" UIValueKey="None"/>
      </ExtendValues>
      <LinkSource BlockId="1">
        <LinkParams>
          <LinkParam ParamId="1" LinkIndex="0"/>
          <LinkParam ParamId="4019" FilterType="InCollection">
            <FilterValueKeys><FilterValueKey>Close</FilterValueKey></FilterValueKeys>
          </LinkParam>
        </LinkParams>
      </LinkSource>
    </ParamDependencyItem>
  </ParamDependencyItems>
  <DialRuleItems>
    <DialRuleItem RuleParamId="1082" ProductType="Conventional">
      <DialParams>
        <DialParam ParamId="17" BlockId="1" CallTypeParamId="46"/>
        <DialParam ParamId="116" CallType="Private"/>
      </DialParams>
    </DialRuleItem>
  </DialRuleItems>
</DataValidationConfig>"#;

    #[test]
    fn parses_references_and_dial_rules() {
        let c = parse_data_validation(DV).unwrap();
        assert_eq!(c.references.len(), 2);
        let rx = &c.references[0];
        assert_eq!((rx.param, rx.block, rx.source_block), (18, Some(1), 607));
        assert_eq!((&rx.locate, &rx.show), (&Locate::TableIndex { start: 1 }, &Show::TableName));
        assert_eq!(rx.special, vec![(0, "None".to_string())]);
        let rrs = &c.references[1];
        assert_eq!(rrs.special, vec![(0, "CurrentChanel".to_string()), (65535, "None".to_string())]);
        assert_eq!(rrs.source_param(0).unwrap().param, 1);
        let filters: Vec<_> = rrs.filters().collect();
        assert_eq!(filters, vec![(4019, &Filter { include: true, keys: vec!["Close".into()] })]);
        assert_eq!(c.dial_params.len(), 2);
        assert_eq!((c.dial_params[0].param, c.dial_params[0].block, c.dial_params[0].call_type_param), (17, Some(1), Some(46)));
        assert_eq!(c.dial_params[1].call_type.as_deref(), Some("Private"));
    }

    #[test]
    fn parses_virtual_params() {
        let xml = r#"<ComplexRuleConfig><ComplexRuleCollection>
            <ComplexRule VirtualParamId="-7" BlockID="619" IndexParamId="1182" TypeParamId="1183" RuleType="0">
              <RelatedBlockCollection>
                <RelatedBlock BlockID="1" TypeParamId="2" TypeValue="0" AliasParamId="1"/>
                <RelatedBlock BlockID="4" TypeParamId="78" TypeValue="1" AliasParamId="77"/>
              </RelatedBlockCollection>
            </ComplexRule></ComplexRuleCollection></ComplexRuleConfig>"#;
        let v = parse_complex_rules(xml).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].id, v[0].block, v[0].index_param, v[0].type_param), (-7, 619, Some(1182), Some(1183)));
        assert_eq!(v[0].related[1], RelatedBlock { block: 4, type_value: Some(1), alias_param: Some(77) });
    }

    #[test]
    fn loads_cps_config_when_installed() {
        let root = RcdbRoot::new(crate::rcdb::DEFAULT_CPS_DATA_DIR);
        if !root.path().join(VALIDATION_FILE).is_file() {
            eprintln!("skipping: CPS install not present");
            return;
        }
        let c = root.validation_config().unwrap();
        // Observed in CPS V1.5.24.000.0003: 37 references, the scan list member virtual parameter.
        assert!(c.references.iter().any(|r| r.param == 1612 && r.special == vec![(0, "CurrentChanel".to_string())]));
        assert!(c.virtual_param(-7).is_some_and(|v| v.index_param == Some(1182)));
        assert!(c.dial_params.iter().any(|d| d.param == 17 && d.block == Some(1)));
    }
}
