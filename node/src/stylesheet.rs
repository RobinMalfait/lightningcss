#![allow(dead_code)]

use napi::{
  bindgen_prelude::*, CallContext, JsNumber, JsObject, JsString, JsUndefined, JsUnknown, NapiValue, Ref,
};
use napi_derive::{js_function, napi};
use parcel_css::{
  declaration::DeclarationBlock,
  media_query::{MediaList, MediaQuery},
  properties::{Property, PropertyId},
  rules::{
    counter_style::{CounterStyleProperty, CounterStyleRule, System},
    font_face::{FontFaceProperty, FontFaceRule},
    font_palette_values::{FontPaletteValuesProperty, FontPaletteValuesRule},
    keyframes::{Keyframe, KeyframesRule},
    style::StyleRule,
    CssRule, CssRuleList,
  },
  stylesheet::{ParserOptions, PrinterOptions, StyleSheet},
  traits::{Parse, ToCss},
};

// https://drafts.csswg.org/cssom/#the-cssstylesheet-interface
#[napi(js_name = "CSSStyleSheet")]
struct CSSStyleSheet {
  stylesheet: StyleSheet<'static>,
  code: Option<String>,
  rules: Option<Reference<CSSRuleList>>,
}

#[napi]
impl CSSStyleSheet {
  #[napi(constructor)]
  pub fn new(env: Env) -> Self {
    CSSGroupingRule::init(env);
    CSSStyleSheet {
      stylesheet: StyleSheet::new(
        vec!["empty.css".into()],
        CssRuleList(Vec::new()),
        ParserOptions::default(),
      ),
      code: None,
      rules: None,
    }
  }

  #[napi]
  pub fn replace_sync(&mut self, env: Env, code: String) -> Result<()> {
    // Disconnect all existing rules from the stylesheet.
    if let Some(rules) = &mut self.rules {
      let rules = &mut **rules;
      for rule in rules.rules.iter_mut() {
        if let Some(rule) = rule {
          let rule: &mut CSSRule = get_reference(env, rule)?;
          rule.disconnect();
        }
      }

      rules.rules.clear();
    }

    // Source string will be owned by the stylesheet, so it'll be freed when the
    // JS garbage collector runs. We need to extend the lifetime to 'static to satisfy Rust.
    let s = extend_lifetime(&code);
    println!("{:p}", s);
    self.code = Some(code);
    self.stylesheet = StyleSheet::parse("style.css", s, ParserOptions::default()).unwrap();
    Ok(())
  }

  #[napi(getter)]
  pub fn css_rules(&mut self, env: Env, reference: Reference<CSSStyleSheet>) -> Result<Reference<CSSRuleList>> {
    if let Some(rules) = &self.rules {
      return rules.clone(env);
    }

    let rules = CSSRuleList {
      owner: RuleListOwner::StyleSheet(extend_lifetime_mut(self)),
      rules: Vec::new(),
      parent_rule: None,
      stylesheet_reference: Some(reference.downgrade()),
    };

    self.rules = Some(CSSRuleList::into_reference(rules, env)?);
    self.rules.as_ref().unwrap().clone(env)
  }

  #[napi]
  pub fn insert_rule(&mut self, env: Env, rule: String, index: Option<u32>) -> Result<u32> {
    insert_rule(&mut self.stylesheet.rules.0, &mut self.rules, env, rule, index)
  }

  #[napi]
  pub fn add_rule(&mut self, env: Env, selector: String, style: String, index: Option<u32>) -> Result<i32> {
    // https://drafts.csswg.org/cssom/#dom-cssstylesheet-addrule

    let string = format!("{} {{ {} }}", selector, style);
    self.insert_rule(env, string, index)?;

    Ok(-1)
  }

  #[napi]
  pub fn delete_rule(&mut self, env: Env, index: u32) -> Result<()> {
    delete_rule(&mut self.stylesheet.rules.0, &mut self.rules, env, index as usize)
  }

  #[napi]
  pub fn remove_rule(&mut self, env: Env, index: u32) -> Result<()> {
    self.delete_rule(env, index)
  }
}

fn insert_rule(
  rules: &mut Vec<CssRule<'static>>,
  js_rules: &mut Option<Reference<CSSRuleList>>,
  env: Env,
  text: String,
  index: Option<u32>,
) -> Result<u32> {
  // https://drafts.csswg.org/cssom/#insert-a-css-rule
  let index = index.unwrap_or(0) as usize;
  if index > rules.len() {
    return Err(napi::Error::new(
      napi::Status::GenericFailure,
      "Index out of bounds".into(),
    ));
  }

  let new_rule = CssRule::parse_string(extend_lifetime(&text), ParserOptions::default()).unwrap();

  // TODO: Check if new_rule can be inserted at position (e.g. @import)

  // Invalidate existing rule indices.
  if let Some(rules) = js_rules {
    let rules = &mut *rules;

    for rule in &rules.rules[index..] {
      if let Some(rule) = rule {
        let rule: &mut CSSRule = get_reference(env, rule)?;
        if let RuleInner::Connected { index, .. } = &mut rule.inner {
          *index += 1;
        }
      }
    }

    rules.rules.insert(index, None);
  }

  rules.insert(index, new_rule);

  if let Some(rules) = js_rules {
    // Store rule text in JS rule object so it is garbage collected.
    rules.set_rule_text(env, index, text)?;
  }

  Ok(index as u32)
}

fn delete_rule<T>(
  rules: &mut Vec<T>,
  js_rules: &mut Option<Reference<CSSRuleList>>,
  env: Env,
  index: usize,
) -> Result<()> {
  // https://drafts.csswg.org/cssom/#remove-a-css-rule
  if index > rules.len() {
    return Err(napi::Error::new(
      napi::Status::GenericFailure,
      "Index out of bounds".into(),
    ));
  }

  if let Some(rule_refs) = js_rules {
    rule_refs.delete_rule(env, index)?;
  }

  rules.remove(index);
  Ok(())
}

fn get_reference<T: napi::bindgen_prelude::FromNapiMutRef>(
  env: Env,
  reference: &Ref<()>,
) -> Result<&'static mut T> {
  let obj: JsUnknown = env.get_reference_value(reference)?;
  unsafe {
    let napi_value = napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), obj)?;
    T::from_napi_mut_ref(env.raw(), napi_value)
  }
}

enum RuleOrKeyframe {
  Rule(CssRule<'static>),
  Keyframe(Keyframe<'static>),
}

enum RuleOrKeyframeRef<'a> {
  Rule(&'a CssRule<'static>),
  Keyframe(&'a Keyframe<'static>),
}

enum RuleOrKeyframeRefMut<'a> {
  Rule(&'a mut CssRule<'static>),
  Keyframe(&'a mut Keyframe<'static>),
}

impl RuleOrKeyframe {
  fn js_value(&self, env: Env, css_rule: CSSRule) -> Result<JsUnknown> {
    match self {
      RuleOrKeyframe::Rule(rule) => css_rule_to_js_unknown(rule, env, css_rule),
      RuleOrKeyframe::Keyframe(_) => keyframe_to_js_unknown(env, css_rule),
    }
  }
}

fn css_rule_to_js_unknown(rule: &CssRule<'static>, env: Env, css_rule: CSSRule) -> Result<JsUnknown> {
  let napi_value = match rule {
    CssRule::Style(_) => {
      let rule = CSSStyleRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Media(_) => {
      let rule = CSSMediaRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Supports(_) => {
      let rule = CSSSupportsRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Keyframes(_) => {
      let rule = CSSKeyframesRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Import(_) => {
      let rule = CSSImportRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Namespace(_) => {
      let rule = CSSNamespaceRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::LayerStatement(_) => {
      let rule = CSSLayerStatementRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::LayerBlock(_) => {
      let rule = CSSLayerBlockRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::Page(_) => {
      let rule = CSSPageRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::FontFace(_) => {
      let rule = CSSFontFaceRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::FontPaletteValues(_) => {
      let rule = CSSFontPaletteValuesRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::CounterStyle(_) => {
      let rule = CSSCounterStyleRule::new(css_rule);
      unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? }
    }
    CssRule::MozDocument(_) => {
      todo!()
    }
    CssRule::Nesting(_) => {
      todo!()
    }
    CssRule::Viewport(_) => {
      todo!()
    }
    CssRule::CustomMedia(_) => {
      todo!()
    }
    CssRule::Property(_) => {
      todo!()
    }
    CssRule::Ignored => {
      unreachable!()
    }
  };

  unsafe { napi::JsUnknown::from_napi_value(env.raw(), napi_value) }
}

fn keyframe_to_js_unknown(env: Env, css_rule: CSSRule) -> Result<JsUnknown> {
  let rule = CSSKeyframeRule::new(css_rule);
  let napi_value = unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(env.raw(), rule)? };
  unsafe { napi::JsUnknown::from_napi_value(env.raw(), napi_value) }
}

enum RuleListReference<'a> {
  Rules(&'a Vec<CssRule<'static>>),
  Keyframes(&'a Vec<Keyframe<'static>>),
}

impl<'a> RuleListReference<'a> {
  fn len(&self) -> usize {
    match self {
      RuleListReference::Rules(r) => r.len(),
      RuleListReference::Keyframes(k) => k.len(),
    }
  }

  fn rule(&self, index: usize) -> RuleOrKeyframeRef<'a> {
    let rule_list = match self {
      RuleListReference::Rules(r) => &**r,
      RuleListReference::Keyframes(keyframes) => return RuleOrKeyframeRef::Keyframe(&keyframes[index]),
    };

    RuleOrKeyframeRef::Rule(&rule_list[index])
  }
}

enum RuleListReferenceMut<'a> {
  Rules(&'a mut Vec<CssRule<'static>>),
  Keyframes(&'a mut Vec<Keyframe<'static>>),
}

impl<'a> RuleListReferenceMut<'a> {
  fn len(&self) -> usize {
    match self {
      RuleListReferenceMut::Rules(r) => r.len(),
      RuleListReferenceMut::Keyframes(k) => k.len(),
    }
  }

  fn rule(&mut self, index: usize) -> RuleOrKeyframeRefMut<'a> {
    let rule_list = match self {
      RuleListReferenceMut::Rules(r) => extend_lifetime_mut(&mut **r),
      RuleListReferenceMut::Keyframes(keyframes) => {
        return RuleOrKeyframeRefMut::Keyframe(extend_lifetime_mut(&mut keyframes[index]))
      }
    };

    RuleOrKeyframeRefMut::Rule(&mut rule_list[index])
  }
}

enum RuleListOwner {
  StyleSheet(&'static mut CSSStyleSheet),
  Rule(&'static mut CSSRule),
}

#[napi(js_name = "CSSRuleList")]
struct CSSRuleList {
  owner: RuleListOwner,
  rules: Vec<Option<Ref<()>>>,
  parent_rule: Option<WeakReference<CSSRule>>,
  stylesheet_reference: Option<WeakReference<CSSStyleSheet>>,
}

#[napi]
impl CSSRuleList {
  #[napi(constructor)]
  pub fn new() -> Self {
    unreachable!()
  }

  fn rule_list<'a>(&'a self) -> RuleListReference<'a> {
    RuleListReference::Rules(match &self.owner {
      RuleListOwner::StyleSheet(stylesheet) => &stylesheet.stylesheet.rules.0,
      RuleListOwner::Rule(rule) => match rule.rule() {
        CssRule::Media(media) => &media.rules.0,
        CssRule::Supports(supports) => &supports.rules.0,
        CssRule::LayerBlock(layer) => &layer.rules.0,
        CssRule::Style(style) => &style.rules.0,
        CssRule::Keyframes(keyframes) => return RuleListReference::Keyframes(&keyframes.keyframes),
        _ => unreachable!(),
      },
    })
  }

  fn rule_list_mut<'a>(&'a mut self) -> RuleListReferenceMut<'a> {
    RuleListReferenceMut::Rules(match &mut self.owner {
      RuleListOwner::StyleSheet(stylesheet) => &mut stylesheet.stylesheet.rules.0,
      RuleListOwner::Rule(rule) => match rule.rule_mut() {
        CssRule::Media(media) => &mut media.rules.0,
        CssRule::Supports(supports) => &mut supports.rules.0,
        CssRule::LayerBlock(layer) => &mut layer.rules.0,
        CssRule::Style(style) => &mut style.rules.0,
        CssRule::Keyframes(keyframes) => return RuleListReferenceMut::Keyframes(&mut keyframes.keyframes),
        _ => unreachable!(),
      },
    })
  }

  fn rule(&self, index: usize) -> RuleOrKeyframeRef {
    self.rule_list().rule(index)
  }

  fn rule_mut(&mut self, index: usize) -> RuleOrKeyframeRefMut {
    self.rule_list_mut().rule(index)
  }

  #[napi(getter)]
  pub fn length(&self) -> u32 {
    self.rule_list().len() as u32
  }

  #[napi]
  pub fn item(&mut self, index: u32, env: Env) -> Result<JsUnknown> {
    let index = index as usize;
    if let Some(Some(rule)) = self.rules.get(index) {
      return env.get_reference_value(rule);
    }

    if index >= self.rule_list().len() {
      return Ok(env.get_null()?.into_unknown());
    }

    let css_rule = CSSRule {
      inner: RuleInner::Connected {
        rule_list: extend_lifetime_mut(self),
        index,
      },
      parent_rule: self.parent_rule.clone(),
      parent_stylesheet: self.stylesheet_reference.clone(),
      text: None,
    };

    let js_value = match self.rule(index) {
      RuleOrKeyframeRef::Rule(rule) => css_rule_to_js_unknown(rule, env, css_rule)?,
      RuleOrKeyframeRef::Keyframe(_) => keyframe_to_js_unknown(env, css_rule)?,
    };

    if self.rules.len() <= index {
      self.rules.resize_with(index + 1, || None);
    }

    self.rules[index] = Some(env.create_reference_with_refcount(&js_value, 0)?);
    Ok(js_value)
  }

  fn set_rule_text(&mut self, env: Env, index: usize, text: String) -> Result<()> {
    // Take ownership of rule text so it is garbage collected when the rule is removed.
    self.item(index as u32, env)?;
    let js_rule: &mut CSSRule = get_reference(env, self.rules[index].as_ref().unwrap())?;
    js_rule.text = Some(text);
    Ok(())
  }

  fn delete_rule(&mut self, env: Env, index: usize) -> Result<()> {
    // https://drafts.csswg.org/cssom/#remove-a-css-rule
    if index < self.rules.len() {
      if let Some(rule) = &self.rules[index] {
        let rule: &mut CSSRule = get_reference(env, rule)?;
        rule.disconnect();
      }

      for rule in &self.rules[index + 1..] {
        if let Some(rule) = rule {
          let rule: &mut CSSRule = get_reference(env, rule)?;
          if let RuleInner::Connected { index, .. } = &mut rule.inner {
            *index -= 1;
          }
        }
      }

      self.rules.remove(index);
    }

    Ok(())
  }
}

enum RuleInner {
  Connected {
    rule_list: &'static mut CSSRuleList,
    index: usize,
  },
  Disconnected(RuleOrKeyframe),
}

impl RuleInner {
  fn rule(&self) -> RuleOrKeyframeRef {
    match self {
      RuleInner::Connected { rule_list, index } => rule_list.rule(*index),
      RuleInner::Disconnected(rule) => match rule {
        RuleOrKeyframe::Rule(rule) => RuleOrKeyframeRef::Rule(rule),
        RuleOrKeyframe::Keyframe(keyframe) => RuleOrKeyframeRef::Keyframe(keyframe),
      },
    }
  }

  fn rule_mut(&mut self) -> RuleOrKeyframeRefMut {
    match self {
      RuleInner::Connected { rule_list, index } => rule_list.rule_mut(*index),
      RuleInner::Disconnected(rule) => match rule {
        RuleOrKeyframe::Rule(rule) => RuleOrKeyframeRefMut::Rule(rule),
        RuleOrKeyframe::Keyframe(keyframe) => RuleOrKeyframeRefMut::Keyframe(keyframe),
      },
    }
  }

  fn disconnect(&mut self) {
    *self = RuleInner::Disconnected(match self.rule() {
      RuleOrKeyframeRef::Rule(rule) => RuleOrKeyframe::Rule(rule.clone()),
      RuleOrKeyframeRef::Keyframe(keyframe) => RuleOrKeyframe::Keyframe(keyframe.clone()),
    })
  }
}

#[napi(js_name = "CSSRule")]
struct CSSRule {
  inner: RuleInner,
  parent_rule: Option<WeakReference<CSSRule>>,
  parent_stylesheet: Option<WeakReference<CSSStyleSheet>>,
  text: Option<String>,
}

#[napi]
impl CSSRule {
  #[napi(constructor)]
  pub fn new() -> Self {
    unreachable!()
  }

  #[napi(getter, js_name = "type")]
  pub fn kind(&self) -> u32 {
    match self.inner.rule() {
      RuleOrKeyframeRef::Rule(rule) => match rule {
        CssRule::Style(..) => 1,
        CssRule::Import(..) => 3,
        CssRule::Media(..) => 4,
        CssRule::FontFace(..) => 5,
        CssRule::Page(..) => 6,
        CssRule::Keyframes(..) => 7,
        CssRule::Namespace(..) => 10,
        CssRule::CounterStyle(..) => 11,
        CssRule::Supports(..) => 12,
        CssRule::MozDocument(..) => 13,
        CssRule::Viewport(..) => 15,
        CssRule::FontPaletteValues(..) => 19,
        _ => 0,
      },
      RuleOrKeyframeRef::Keyframe(_) => 8,
    }
  }

  #[napi(getter)]
  pub fn css_text(&self) -> String {
    match self.inner.rule() {
      RuleOrKeyframeRef::Rule(rule) => rule.to_css_string(PrinterOptions::default()).unwrap(),
      RuleOrKeyframeRef::Keyframe(rule) => rule.to_css_string(PrinterOptions::default()).unwrap(),
    }
  }

  #[napi(setter)]
  pub fn set_css_text(&self) {
    // On setting the cssText attribute must do nothing.
  }

  fn rule(&self) -> &CssRule<'static> {
    match self.inner.rule() {
      RuleOrKeyframeRef::Rule(rule) => rule,
      _ => unreachable!(),
    }
  }

  fn rule_mut(&mut self) -> &mut CssRule<'static> {
    match self.inner.rule_mut() {
      RuleOrKeyframeRefMut::Rule(rule) => rule,
      _ => unreachable!(),
    }
  }

  #[napi(getter)]
  pub fn parent_style_sheet(&self) -> Option<WeakReference<CSSStyleSheet>> {
    self.parent_stylesheet.clone()
  }

  #[napi(getter)]
  pub fn parent_rule(&self) -> Option<WeakReference<CSSRule>> {
    self.parent_rule.clone()
  }

  fn disconnect(&mut self) {
    self.inner.disconnect();
    self.parent_rule = None;
    self.parent_stylesheet = None;
  }
}

#[napi(js_name = "CSSStyleRule")]
struct CSSStyleRule {
  rule: CSSRule,
  style: Option<Reference<CSSStyleDeclaration>>,
  selector_text: Option<String>,
  rules: Option<Reference<CSSRuleList>>,
}

#[napi]
impl CSSStyleRule {
  fn new(rule: CSSRule) -> CSSStyleRule {
    CSSStyleRule {
      rule,
      style: None,
      selector_text: None,
      rules: None,
    }
  }

  #[napi(constructor)]
  pub fn constructor() {
    unreachable!();
  }

  #[napi(getter)]
  pub fn selector_text(&self) -> String {
    match &*self.rule.rule() {
      CssRule::Style(style) => cssparser::ToCss::to_css_string(&style.selectors),
      _ => unreachable!(),
    }
  }

  #[napi(setter)]
  pub fn set_selector_text(&mut self, text: String) {
    let s = extend_lifetime(&text);
    self.selector_text = Some(text);
    self.rule_mut().set_selector_text(s).unwrap();
  }

  #[napi(getter)]
  pub fn style(&mut self, env: Env, reference: Reference<CSSStyleRule>) -> Result<Reference<CSSStyleDeclaration>> {
    if let Some(style) = &self.style {
      return style.clone(env);
    }

    let parent_rule =
      unsafe { std::mem::transmute::<WeakReference<CSSStyleRule>, WeakReference<CSSRule>>(reference.downgrade()) };
    let style = CSSStyleDeclaration::into_reference(CSSStyleDeclaration::new(parent_rule), env)?;
    self.style = Some(style.clone(env)?);
    Ok(style)
  }

  #[napi(setter)]
  pub fn set_style(&mut self, text: String, env: Env, reference: Reference<CSSStyleRule>) -> Result<()> {
    self.style(env, reference)?.set_css_text(text, env)?;
    Ok(())
  }

  // https://drafts.csswg.org/css-nesting-1/#cssom-style
  #[napi(getter)]
  pub fn css_rules(&mut self, env: Env, reference: Reference<CSSStyleRule>) -> Result<Reference<CSSRuleList>> {
    if let Some(rules) = &self.rules {
      return rules.clone(env);
    }

    let rules = CSSRuleList {
      owner: RuleListOwner::Rule(extend_lifetime_mut(&mut self.rule)),
      rules: Vec::new(),
      parent_rule: Some(unsafe {
        std::mem::transmute::<WeakReference<CSSStyleRule>, WeakReference<CSSRule>>(reference.downgrade())
      }),
      stylesheet_reference: self.rule.parent_stylesheet.clone(),
    };

    self.rules = Some(CSSRuleList::into_reference(rules, env)?);
    self.rules.as_ref().unwrap().clone(env)
  }

  #[napi]
  pub fn insert_rule(&mut self, env: Env, text: String, index: Option<u32>) -> Result<u32> {
    let rule = self.rule.rule_mut();
    let style = match rule {
      CssRule::Style(style) => style,
      CssRule::Nesting(nesting) => &mut nesting.style,
      _ => unreachable!(),
    };
    // TODO: needs to handle nesting selectors and other context...
    insert_rule(&mut style.rules.0, &mut self.rules, env, text, index)
  }

  #[napi]
  pub fn delete_rule(&mut self, env: Env, index: u32) -> Result<()> {
    let rule = self.rule.rule_mut();
    let style = match rule {
      CssRule::Style(style) => style,
      CssRule::Nesting(nesting) => &mut nesting.style,
      _ => unreachable!(),
    };
    delete_rule(&mut style.rules.0, &mut self.rules, env, index as usize)
  }

  fn rule(&self) -> &StyleRule<'static> {
    let rule = self.rule.rule();
    match rule {
      CssRule::Style(style) => style,
      CssRule::Nesting(nesting) => &nesting.style,
      _ => unreachable!(),
    }
  }

  fn rule_mut(&mut self) -> &mut StyleRule<'static> {
    let rule = self.rule.rule_mut();
    match rule {
      CssRule::Style(style) => style,
      CssRule::Nesting(nesting) => &mut nesting.style,
      _ => unreachable!(),
    }
  }
}

trait Declarations {
  fn get_longhands(&mut self) -> Result<Vec<String>>;
  fn get_property_value(&mut self, property: &str) -> Result<String>;
  fn get_property_priority(&mut self, property: &str) -> Result<&str>;
  fn set_property(&mut self, property: &'static str, value: &'static str, priority: Option<String>) -> Result<()>;
  fn remove_property(&mut self, property: &str) -> Result<()>;
  fn css_text(&self) -> Result<String>;
  fn set_css_text(&mut self, s: &'static str) -> Result<()>;
}

impl Declarations for DeclarationBlock<'static> {
  fn get_longhands(&mut self) -> Result<Vec<String>> {
    let mut longhands = Vec::new();
    for (property, _important) in self.iter() {
      let property_id = property.property_id();
      if let Some(properties) = property_id.longhands() {
        longhands.extend(properties.iter().map(|property_id| property_id.name().to_owned()))
      } else {
        longhands.push(property_id.name().to_owned());
      }
    }

    return Ok(longhands);
  }

  fn get_property_value(&mut self, property: &str) -> Result<String> {
    let property_id = PropertyId::parse_string(property).unwrap();
    let opts = PrinterOptions::default();

    if let Some((value, _important)) = self.get(&property_id) {
      return Ok(value.value_to_css_string(opts).unwrap());
    }

    Ok(String::new())
  }

  fn get_property_priority(&mut self, property: &str) -> Result<&str> {
    let property_id = PropertyId::parse_string(property).unwrap();
    let important = if let Some((_value, important)) = self.get(&property_id) {
      important
    } else {
      false
    };

    Ok(if important { "important" } else { "" })
  }

  fn set_property(&mut self, property: &'static str, value: &'static str, priority: Option<String>) -> Result<()> {
    if value.is_empty() {
      self.remove_property(property)?;
      return Ok(());
    }

    let property = Property::parse_string(property.into(), value, ParserOptions::default()).unwrap();
    self.set(
      property,
      if let Some(priority) = priority {
        priority.eq_ignore_ascii_case("important")
      } else {
        false
      },
    );

    Ok(())
  }

  fn remove_property(&mut self, property: &str) -> Result<()> {
    let property_id = PropertyId::parse_string(property).unwrap();
    self.remove(&property_id);
    Ok(())
  }

  fn css_text(&self) -> Result<String> {
    let res = self.to_css_string(PrinterOptions::default()).unwrap();
    Ok(res)
  }

  fn set_css_text(&mut self, s: &'static str) -> Result<()> {
    *self = DeclarationBlock::parse_string(s, ParserOptions::default()).unwrap();
    Ok(())
  }
}

impl Declarations for FontFaceRule<'static> {
  fn get_longhands(&mut self) -> Result<Vec<String>> {
    Ok(self.properties.iter().map(|property| property.name().to_owned()).collect())
  }

  fn get_property_value(&mut self, property: &str) -> Result<String> {
    Ok(
      self
        .properties
        .iter()
        .rev()
        .find(|p| p.name().eq_ignore_ascii_case(property))
        .map_or(String::new(), |property| {
          property.value_to_css_string(PrinterOptions::default()).unwrap()
        }),
    )
  }

  fn get_property_priority(&mut self, _property: &str) -> Result<&str> {
    Ok("")
  }

  fn set_property(
    &mut self,
    property: &'static str,
    value: &'static str,
    _priority: Option<String>,
  ) -> Result<()> {
    let value = FontFaceProperty::parse_string(property, value).unwrap();

    for property in self.properties.iter_mut() {
      if property.name() == value.name() {
        *property = value;
        return Ok(());
      }
    }

    self.properties.push(value);
    Ok(())
  }

  fn remove_property(&mut self, property: &str) -> Result<()> {
    self.properties.retain(|p| !p.name().eq_ignore_ascii_case(property));
    Ok(())
  }

  fn css_text(&self) -> Result<String> {
    Ok(self.declarations_to_css_string(PrinterOptions::default()).unwrap())
  }

  fn set_css_text(&mut self, s: &'static str) -> Result<()> {
    self.properties = FontFaceRule::parse_declarations_string(s).unwrap();
    Ok(())
  }
}

#[napi(js_name = "CSSStyleDeclaration")]
struct CSSStyleDeclaration {
  parent_rule: WeakReference<CSSRule>,
  strings: Vec<String>,
}

#[napi]
impl CSSStyleDeclaration {
  #[napi(constructor)]
  pub fn constructor() -> Self {
    unreachable!()
  }

  fn new(parent_rule: WeakReference<CSSRule>) -> Self {
    CSSStyleDeclaration {
      parent_rule,
      strings: Vec::new(),
    }
  }

  fn declarations(&mut self, env: Env) -> Result<&'static mut dyn Declarations> {
    let mut rule = self.parent_rule.upgrade(env)?.unwrap();
    match rule.inner.rule_mut() {
      RuleOrKeyframeRefMut::Rule(rule) => match rule {
        CssRule::Style(style) => Ok(extend_lifetime_mut(&mut style.declarations)),
        CssRule::Page(page) => Ok(extend_lifetime_mut(&mut page.declarations)),
        CssRule::FontFace(font_face) => Ok(extend_lifetime_mut(font_face)),
        _ => unreachable!(),
      },
      RuleOrKeyframeRefMut::Keyframe(keyframe) => Ok(extend_lifetime_mut(&mut keyframe.declarations)),
    }
  }

  fn add_string(&mut self, string: String) -> &'static str {
    let res = extend_lifetime(&string);
    self.strings.push(string);
    res
  }

  #[napi(getter)]
  pub fn parent_rule(&self) -> WeakReference<CSSRule> {
    self.parent_rule.clone()
  }

  #[napi(getter)]
  pub fn css_text(&mut self, env: Env) -> Result<String> {
    self.declarations(env)?.css_text()
  }

  #[napi(setter)]
  pub fn set_css_text(&mut self, text: String, env: Env) -> Result<()> {
    let s = self.add_string(text);
    self.declarations(env)?.set_css_text(s)
  }

  #[napi(getter)]
  pub fn length(&mut self, env: Env) -> Result<u32> {
    Ok(self.declarations(env)?.get_longhands()?.len() as u32)
  }

  #[napi]
  pub fn item(&mut self, index: u32, env: Env) -> Result<String> {
    let mut longhands = self.declarations(env)?.get_longhands()?;
    let index = index as usize;
    if index < longhands.len() {
      return Ok(std::mem::take(&mut longhands[index]));
    }
    Ok(String::new())
  }

  #[napi]
  pub fn get_property_value(&mut self, property: String, env: Env) -> Result<String> {
    self.declarations(env)?.get_property_value(&property)
  }

  #[napi]
  pub fn get_property_priority(&mut self, property: String, env: Env) -> Result<&str> {
    self.declarations(env)?.get_property_priority(&property)
  }

  #[napi]
  pub fn set_property(
    &mut self,
    property: String,
    value: String,
    priority: Option<String>,
    env: Env,
  ) -> Result<()> {
    let property = self.add_string(property);
    let value = self.add_string(value);
    self.declarations(env)?.set_property(property, value, priority)
  }

  #[napi]
  pub fn remove_property(&mut self, property: String, env: Env) -> Result<String> {
    let declarations = self.declarations(env)?;
    let value = declarations.get_property_value(&property)?;
    declarations.remove_property(&property)?;
    Ok(value)
  }
}

// https://drafts.csswg.org/cssom-1/#the-cssgroupingrule-interface
#[napi(js_name = "CSSGroupingRule")]
struct CSSGroupingRule {
  rule: CSSRule,
  rules: Option<Reference<CSSRuleList>>,
}

#[napi]
impl CSSGroupingRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    CSSGroupingRule { rule, rules: None }
  }

  fn init(env: Env) {
    let constructor_value = napi::bindgen_prelude::get_class_constructor("CSSGroupingRule\0").unwrap();
    let mut value = std::ptr::null_mut();
    unsafe { napi::sys::napi_get_reference_value(env.raw(), constructor_value, &mut value) };
    let constructor = unsafe { JsFunction::from_raw(env.raw(), value).unwrap() };
    let constructor = constructor.coerce_to_object().unwrap();
    let mut prototype: JsObject = constructor.get_named_property("prototype").unwrap();
    prototype
      .set_named_property(
        "insertRule",
        env.create_function("insertRule", grouping_rule_insert).unwrap(),
      )
      .unwrap();
    prototype
      .set_named_property(
        "deleteRule",
        env.create_function("deleteRule", grouping_rule_delete).unwrap(),
      )
      .unwrap();
  }

  #[napi(getter)]
  pub fn css_rules(&mut self, env: Env, reference: Reference<CSSGroupingRule>) -> Result<Reference<CSSRuleList>> {
    if let Some(rules) = &self.rules {
      return rules.clone(env);
    }

    let rules = CSSRuleList {
      owner: RuleListOwner::Rule(extend_lifetime_mut(&mut self.rule)),
      rules: Vec::new(),
      parent_rule: Some(unsafe {
        std::mem::transmute::<WeakReference<CSSGroupingRule>, WeakReference<CSSRule>>(reference.downgrade())
      }),
      stylesheet_reference: self.rule.parent_stylesheet.clone(),
    };

    self.rules = Some(CSSRuleList::into_reference(rules, env)?);
    self.rules.as_ref().unwrap().clone(env)
  }
}

// Inheritance doesn't work with methods. v8 throws "Illegal invocation" errors due to signature checks.
// https://github.com/nodejs/node-addon-api/issues/246
// Instead, define a pure JS function here and assign it to the prototype of the class manually.
#[js_function(2)]
fn grouping_rule_insert(ctx: CallContext) -> Result<JsNumber> {
  let this: JsObject = ctx.this()?;
  // This is probably extremely unsafe.
  // TODO: use napi_type_tag_object?
  let napi_value = unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(ctx.env.raw(), this).unwrap() };
  let rule = unsafe { CSSGroupingRule::from_napi_mut_ref(ctx.env.raw(), napi_value).unwrap() };
  let rules = match rule.rule.rule_mut() {
    CssRule::Media(media) => &mut media.rules.0,
    CssRule::Supports(supports) => &mut supports.rules.0,
    _ => unreachable!(),
  };
  let new_rule: JsString = ctx.get(0)?;
  let utf8 = new_rule.into_utf8()?;
  let new_rule = utf8.into_owned()?;
  let index: Option<u32> = if ctx.length > 1 {
    Some(ctx.get::<JsNumber>(1)?.get_uint32()?)
  } else {
    None
  };
  let res = insert_rule(rules, &mut rule.rules, *ctx.env, new_rule, index);
  if let Ok(res) = res {
    ctx.env.create_uint32(res)
  } else {
    Err(res.unwrap_err())
  }
}

#[js_function(1)]
fn grouping_rule_delete(ctx: CallContext) -> Result<JsUndefined> {
  let this: JsObject = ctx.this()?;
  // This is probably extremely unsafe.
  // TODO: use napi_type_tag_object?
  let napi_value = unsafe { napi::bindgen_prelude::ToNapiValue::to_napi_value(ctx.env.raw(), this).unwrap() };
  let rule = unsafe { CSSGroupingRule::from_napi_mut_ref(ctx.env.raw(), napi_value).unwrap() };
  let rules = match rule.rule.rule_mut() {
    CssRule::Media(media) => &mut media.rules.0,
    CssRule::Supports(supports) => &mut supports.rules.0,
    _ => unreachable!(),
  };
  let index = ctx.get::<JsNumber>(0)?.get_uint32()?;
  delete_rule(rules, &mut rule.rules, *ctx.env, index as usize)?;
  ctx.env.get_undefined()
}

// https://drafts.csswg.org/css-conditional-3/#cssconditionrule
#[napi(js_name = "CSSConditionRule")]
struct CSSConditionRule {
  rule: CSSGroupingRule,
  condition_text: Option<String>,
}

#[napi]
impl CSSConditionRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    CSSConditionRule {
      rule: CSSGroupingRule::new(rule),
      condition_text: None,
    }
  }

  #[napi(getter)]
  pub fn condition_text(&self) -> Result<String> {
    match self.rule.rule.rule() {
      CssRule::Media(media) => Ok(media.query.to_css_string(PrinterOptions::default()).unwrap()),
      CssRule::Supports(supports) => Ok(supports.condition.to_css_string(PrinterOptions::default()).unwrap()),
      _ => Err(napi::Error::new(
        napi::Status::InvalidArg,
        "Not a conditional rule".into(),
      )),
    }
  }

  #[napi(setter)]
  pub fn set_condition_text(&mut self, text: String) {
    match self.rule.rule.rule_mut() {
      CssRule::Media(media) => {
        let s = extend_lifetime(&text);
        self.condition_text = Some(text);
        if let Ok(media_list) = MediaList::parse_string(s) {
          media.query = media_list;
        }
      }
      CssRule::Supports(_) => {
        // Spec doesn't say this can be set. WebKit does nothing, Firefox throws. We do nothing.
      }
      _ => {}
    }
  }
}

// https://drafts.csswg.org/css-conditional-3/#cssmediarule
#[napi(js_name = "CSSMediaRule")]
struct CSSMediaRule {
  rule: CSSConditionRule,
  media: Option<Reference<JSMediaList>>,
}

#[napi]
impl CSSMediaRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule: CSSConditionRule::new(rule),
      media: None,
    }
  }

  #[napi(getter)]
  pub fn media(&mut self, env: Env, reference: Reference<CSSMediaRule>) -> Result<Reference<JSMediaList>> {
    if let Some(media) = &self.media {
      return media.clone(env);
    }

    let reference =
      unsafe { std::mem::transmute::<WeakReference<CSSMediaRule>, WeakReference<CSSRule>>(reference.downgrade()) };

    let media = JSMediaList::into_reference(
      JSMediaList {
        owner: reference,
        strings: Vec::new(),
      },
      env,
    )?;
    self.media = Some(media.clone(env)?);
    Ok(media)
  }

  #[napi(setter)]
  pub fn set_media(&mut self, text: String) {
    self.rule.set_condition_text(text)
  }
}

#[napi(js_name = "MediaList")]
struct JSMediaList {
  owner: WeakReference<CSSRule>,
  strings: Vec<String>,
}

#[napi]
impl JSMediaList {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(owner: WeakReference<CSSRule>) -> Self {
    JSMediaList {
      owner,
      strings: Vec::new(),
    }
  }

  fn media_list(&mut self, env: Env) -> Result<&mut MediaList<'static>> {
    let mut rule = self.owner.upgrade(env)?.unwrap();
    match rule.rule_mut() {
      CssRule::Media(media) => Ok(extend_lifetime_mut(&mut media.query)),
      CssRule::Import(import) => Ok(extend_lifetime_mut(&mut import.media)),
      _ => unreachable!(),
    }
  }

  fn add_string(&mut self, string: String) -> &'static str {
    let s = extend_lifetime(&string);
    self.strings.push(string);
    s
  }

  #[napi(getter)]
  pub fn media_text(&mut self, env: Env) -> Result<String> {
    Ok(self.media_list(env)?.to_css_string(PrinterOptions::default()).unwrap())
  }

  #[napi(setter)]
  pub fn set_media_text(&mut self, text: String, env: Env) -> Result<()> {
    if let Ok(media_list) = MediaList::parse_string(self.add_string(text)) {
      let media = self.media_list(env)?;
      *media = media_list;
    }
    Ok(())
  }

  #[napi(getter)]
  pub fn length(&mut self, env: Env) -> Result<u32> {
    Ok(self.media_list(env)?.media_queries.len() as u32)
  }

  #[napi]
  pub fn item(&mut self, index: u32, env: Env) -> Result<Option<String>> {
    if let Some(query) = self.media_list(env)?.media_queries.get(index as usize) {
      return Ok(Some(query.to_css_string(PrinterOptions::default()).unwrap()));
    }

    Ok(None)
  }

  #[napi]
  pub fn append_medium(&mut self, medium: String, env: Env) -> Result<()> {
    if let Ok(query) = MediaQuery::parse_string(self.add_string(medium)) {
      let media_list = self.media_list(env)?;
      if media_list.media_queries.contains(&query) {
        return Ok(());
      }

      media_list.media_queries.push(query);
    }
    Ok(())
  }

  #[napi]
  pub fn delete_medium(&mut self, medium: String, env: Env) -> Result<()> {
    if let Ok(query) = MediaQuery::parse_string(&medium) {
      let media_list = self.media_list(env)?;
      let queries = &mut media_list.media_queries;
      let len = queries.len();
      queries.retain(|q| *q != query);
      if queries.len() == len {
        return Err(napi::Error::new(napi::Status::GenericFailure, "Rule not found".into()));
      }
    }

    Ok(())
  }
}

// https://drafts.csswg.org/css-conditional-3/#the-csssupportsrule-interface
#[napi(js_name = "CSSSupportsRule")]
struct CSSSupportsRule {
  rule: CSSConditionRule,
}

#[napi]
impl CSSSupportsRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule: CSSConditionRule::new(rule),
    }
  }
}

// https://drafts.csswg.org/css-animations-1/#csskeyframesrule
#[napi(js_name = "CSSKeyframesRule")]
struct CSSKeyframesRule {
  rule: CSSRule,
  rules: Option<Reference<CSSRuleList>>,
}

#[napi]
impl CSSKeyframesRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule, rules: None }
  }

  fn rule(&self) -> Result<&KeyframesRule<'static>> {
    match self.rule.rule() {
      CssRule::Keyframes(rule) => Ok(rule),
      _ => {
        return Err(napi::Error::new(
          napi::Status::GenericFailure,
          "Not an @keyframes rule".into(),
        ))
      }
    }
  }

  fn rule_mut(&mut self) -> Result<&mut KeyframesRule<'static>> {
    match self.rule.rule_mut() {
      CssRule::Keyframes(rule) => Ok(rule),
      _ => {
        return Err(napi::Error::new(
          napi::Status::GenericFailure,
          "Not an @keyframes rule".into(),
        ))
      }
    }
  }

  #[napi(getter)]
  pub fn name(&self) -> Result<&str> {
    Ok(self.rule()?.name.0.as_ref())
  }

  #[napi(setter)]
  pub fn set_name(&mut self, name: String) -> Result<()> {
    let rule = self.rule_mut()?;
    rule.name.0 = name.into();
    Ok(())
  }

  #[napi(getter)]
  pub fn css_rules(&mut self, env: Env, reference: Reference<CSSKeyframesRule>) -> Result<Reference<CSSRuleList>> {
    if let Some(rules) = &self.rules {
      return rules.clone(env);
    }

    let rules = CSSRuleList {
      owner: RuleListOwner::Rule(extend_lifetime_mut(&mut self.rule)),
      rules: Vec::new(),
      parent_rule: Some(unsafe {
        std::mem::transmute::<WeakReference<CSSKeyframesRule>, WeakReference<CSSRule>>(reference.downgrade())
      }),
      stylesheet_reference: self.rule.parent_stylesheet.clone(),
    };

    self.rules = Some(CSSRuleList::into_reference(rules, env)?);
    self.rules.as_ref().unwrap().clone(env)
  }

  fn find_index(&self, select: String) -> Result<Option<usize>> {
    let parsed = match Vec::parse_string(&select) {
      Ok(selector) => selector,
      Err(_) => return Ok(None),
    };

    // Find the _last_ matching rule.
    let rule = self.rule()?;
    let len = rule.keyframes.len();
    match rule.keyframes.iter().rev().position(|keyframe| keyframe.selectors == parsed) {
      Some(index) => Ok(Some(len - 1 - index)),
      _ => Ok(None),
    }
  }

  #[napi]
  pub fn find_rule(
    &mut self,
    select: String,
    env: Env,
    reference: Reference<CSSKeyframesRule>,
  ) -> Result<JsUnknown> {
    match self.find_index(select)? {
      Some(index) => self.css_rules(env, reference)?.item(index as u32, env),
      None => Ok(env.get_null()?.into_unknown()),
    }
  }

  #[napi]
  pub fn append_rule(&mut self, text: String, env: Env, reference: Reference<CSSKeyframesRule>) -> Result<()> {
    if let Ok(keyframe) = Keyframe::parse_string(extend_lifetime(&text)) {
      let rule = self.rule_mut()?;
      let index = rule.keyframes.len();
      rule.keyframes.push(keyframe);

      // Take ownership of rule text so it is garbage collected when the rule is removed.
      let mut rule_list = self.css_rules(env, reference)?;
      rule_list.set_rule_text(env, index, text)?;
    }

    Ok(())
  }

  #[napi]
  pub fn delete_rule(&mut self, select: String, env: Env) -> Result<()> {
    match self.find_index(select)? {
      Some(index) => {
        let rule = match self.rule.rule_mut() {
          CssRule::Keyframes(rule) => rule,
          _ => unreachable!(),
        };
        delete_rule(&mut rule.keyframes, &mut self.rules, env, index)?;
      }
      None => {}
    }

    Ok(())
  }
}

// https://drafts.csswg.org/css-animations-1/#csskeyframerule
#[napi(js_name = "CSSKeyframeRule")]
struct CSSKeyframeRule {
  rule: CSSRule,
  style: Option<Reference<CSSStyleDeclaration>>,
  key_text: Option<String>,
}

#[napi]
impl CSSKeyframeRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule,
      style: None,
      key_text: None,
    }
  }

  #[napi(getter)]
  pub fn key_text(&self) -> String {
    match self.rule.inner.rule() {
      RuleOrKeyframeRef::Keyframe(keyframe) => {
        keyframe.selectors.to_css_string(PrinterOptions::default()).unwrap()
      }
      _ => unreachable!(),
    }
  }

  #[napi(setter)]
  pub fn set_key_text(&mut self, text: String) {
    if let Ok(selectors) = Vec::parse_string(extend_lifetime(&text)) {
      match self.rule.inner.rule_mut() {
        RuleOrKeyframeRefMut::Keyframe(keyframe) => {
          keyframe.selectors = selectors;
          self.key_text = Some(text);
        }
        _ => unreachable!(),
      }
    } else {
      // Spec says to throw a SyntaxError, but no browser does?
    }
  }

  #[napi(getter)]
  pub fn style(
    &mut self,
    env: Env,
    reference: Reference<CSSKeyframeRule>,
  ) -> Result<Reference<CSSStyleDeclaration>> {
    if let Some(style) = &self.style {
      return style.clone(env);
    }

    let parent_rule = unsafe {
      std::mem::transmute::<WeakReference<CSSKeyframeRule>, WeakReference<CSSRule>>(reference.downgrade())
    };

    let style = CSSStyleDeclaration::into_reference(CSSStyleDeclaration::new(parent_rule), env)?;
    self.style = Some(style.clone(env)?);
    Ok(style)
  }

  #[napi(setter)]
  pub fn set_style(&mut self, text: String, env: Env, reference: Reference<CSSKeyframeRule>) -> Result<()> {
    self.style(env, reference)?.set_css_text(text, env)?;
    Ok(())
  }
}

// https://drafts.csswg.org/cssom-1/#the-cssimportrule-interface
#[napi(js_name = "CSSImportRule")]
struct CSSImportRule {
  rule: CSSRule,
  media: Option<Reference<JSMediaList>>,
}

#[napi]
impl CSSImportRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule, media: None }
  }

  #[napi(getter)]
  pub fn href(&self) -> &str {
    match self.rule.rule() {
      CssRule::Import(import) => import.url.as_ref(),
      _ => unreachable!(),
    }
  }

  #[napi(getter)]
  pub fn media(&mut self, env: Env, reference: Reference<CSSImportRule>) -> Result<Reference<JSMediaList>> {
    if let Some(media) = &self.media {
      return media.clone(env);
    }

    let reference = unsafe {
      std::mem::transmute::<WeakReference<CSSImportRule>, WeakReference<CSSRule>>(reference.downgrade())
    };

    let media = JSMediaList::into_reference(
      JSMediaList {
        owner: reference,
        strings: Vec::new(),
      },
      env,
    )?;
    self.media = Some(media.clone(env)?);
    Ok(media)
  }

  #[napi(setter)]
  pub fn set_media(&mut self, media: String, env: Env, reference: Reference<CSSImportRule>) -> Result<()> {
    self.media(env, reference)?.set_media_text(media, env)
  }

  // https://drafts.csswg.org/css-cascade-5/#extensions-to-cssimportrule-interface
  #[napi(getter)]
  pub fn layer_name(&self) -> Option<String> {
    match self.rule.rule() {
      CssRule::Import(import) => match &import.layer {
        Some(Some(layer_name)) => Some(layer_name.to_css_string(PrinterOptions::default()).unwrap()),
        Some(None) => Some("".into()),
        None => None,
      },
      _ => None,
    }
  }

  // TODO: no way to read/update supports in spec?
  // TODO: why is layerName read only?
  // TODO: styleSheet?
}

#[napi(js_name = "CSSNamespaceRule")]
struct CSSNamespaceRule {
  rule: CSSRule,
}

#[napi]
impl CSSNamespaceRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule }
  }

  #[napi(getter, js_name = "namespaceURI")]
  pub fn namespace_uri(&self) -> &str {
    match self.rule.rule() {
      CssRule::Namespace(namespace) => namespace.url.as_ref(),
      _ => unreachable!(),
    }
  }

  #[napi(getter)]
  pub fn prefix(&self) -> &str {
    match self.rule.rule() {
      CssRule::Namespace(namespace) => match &namespace.prefix {
        Some(prefix) => prefix.as_ref(),
        None => "",
      },
      _ => unreachable!(),
    }
  }
}

#[napi(js_name = "CSSLayerStatementRule")]
struct CSSLayerStatementRule {
  rule: CSSRule,
}

#[napi]
impl CSSLayerStatementRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule }
  }

  #[napi(getter)]
  pub fn name_list(&self) -> Vec<String> {
    // TODO: SameObject? Safari: false, Firefox: true
    // TODO: freeze
    match self.rule.rule() {
      CssRule::LayerStatement(layer) => layer
        .names
        .iter()
        .map(|name| name.to_css_string(PrinterOptions::default()).unwrap())
        .collect(),
      _ => unreachable!(),
    }
  }
}

#[napi(js_name = "CSSLayerBlockRule")]
struct CSSLayerBlockRule {
  rule: CSSGroupingRule,
}

#[napi]
impl CSSLayerBlockRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule: CSSGroupingRule::new(rule),
    }
  }

  #[napi(getter)]
  pub fn name(&self) -> String {
    match self.rule.rule.rule() {
      CssRule::LayerBlock(layer) => match &layer.name {
        Some(name) => name.to_css_string(PrinterOptions::default()).unwrap(),
        _ => "".into(),
      },
      _ => unreachable!(),
    }
  }
}

#[napi(js_name = "CSSPageRule")]
struct CSSPageRule {
  rule: CSSGroupingRule,
  style: Option<Reference<CSSStyleDeclaration>>,
  selector_text: Option<String>,
}

#[napi]
impl CSSPageRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule: CSSGroupingRule::new(rule),
      style: None,
      selector_text: None,
    }
  }

  #[napi(getter)]
  pub fn selector_text(&self) -> Result<String> {
    // TODO: Safari returns "@page :first", Chrome returns ":first", Firefox doesn't support selectorText.
    match self.rule.rule.rule() {
      CssRule::Page(page) => Ok(page.selectors.to_css_string(PrinterOptions::default()).unwrap()),
      _ => Err(napi::Error::new(
        napi::Status::GenericFailure,
        "Not an @page rule".into(),
      )),
    }
  }

  #[napi(setter)]
  pub fn set_selector_text(&mut self, text: String) -> Result<()> {
    // TODO: Chrome and Safari both seem not to implement this? But it's in the spec...
    match self.rule.rule.rule_mut() {
      CssRule::Page(page) => {
        page.selectors = Vec::parse_string(extend_lifetime(&text)).unwrap();
        self.selector_text = Some(text);
        Ok(())
      }
      _ => Err(napi::Error::new(
        napi::Status::GenericFailure,
        "Not an @page rule".into(),
      )),
    }
  }

  #[napi(getter)]
  pub fn style(&mut self, env: Env, reference: Reference<CSSPageRule>) -> Result<Reference<CSSStyleDeclaration>> {
    if let Some(style) = &self.style {
      return style.clone(env);
    }

    let parent_rule =
      unsafe { std::mem::transmute::<WeakReference<CSSPageRule>, WeakReference<CSSRule>>(reference.downgrade()) };
    let style = CSSStyleDeclaration::into_reference(CSSStyleDeclaration::new(parent_rule), env)?;
    self.style = Some(style.clone(env)?);
    Ok(style)
  }

  #[napi(setter)]
  pub fn set_style(&mut self, text: String, env: Env, reference: Reference<CSSPageRule>) -> Result<()> {
    self.style(env, reference)?.set_css_text(text, env)?;
    Ok(())
  }
}

// https://drafts.csswg.org/css-nesting-1/#cssom-nesting
#[napi(js_name = "CSSNestingRule")]
struct CSSNestingRule {
  rule: CSSStyleRule,
}

#[napi]
impl CSSNestingRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule: CSSStyleRule::new(rule),
    }
  }

  #[napi(getter)]
  pub fn selector_text(&self) -> String {
    self.rule.selector_text()
  }

  #[napi(setter)]
  pub fn set_selector_text(&mut self, text: String) {
    self.rule.set_selector_text(text)
  }

  #[napi(getter)]
  pub fn style(
    &mut self,
    env: Env,
    reference: Reference<CSSNestingRule>,
  ) -> Result<Reference<CSSStyleDeclaration>> {
    let reference =
      unsafe { std::mem::transmute::<Reference<CSSNestingRule>, Reference<CSSStyleRule>>(reference) };
    self.rule.style(env, reference)
  }

  #[napi(setter)]
  pub fn set_style(&mut self, text: String, env: Env, reference: Reference<CSSNestingRule>) -> Result<()> {
    let reference =
      unsafe { std::mem::transmute::<Reference<CSSNestingRule>, Reference<CSSStyleRule>>(reference) };
    self.rule.set_style(text, env, reference)
  }

  #[napi]
  pub fn insert_rule(&mut self, env: Env, text: String, index: Option<u32>) -> Result<u32> {
    self.rule.insert_rule(env, text, index)
  }

  #[napi]
  pub fn delete_rule(&mut self, env: Env, index: u32) -> Result<()> {
    self.rule.delete_rule(env, index)
  }
}

#[napi(js_name = "CSSFontFaceRule")]
struct CSSFontFaceRule {
  rule: CSSRule,
  style: Option<Reference<CSSStyleDeclaration>>,
}

#[napi]
impl CSSFontFaceRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule, style: None }
  }

  #[napi(getter)]
  pub fn style(
    &mut self,
    env: Env,
    reference: Reference<CSSFontFaceRule>,
  ) -> Result<Reference<CSSStyleDeclaration>> {
    if let Some(style) = &self.style {
      return style.clone(env);
    }

    let parent_rule = unsafe {
      std::mem::transmute::<WeakReference<CSSFontFaceRule>, WeakReference<CSSRule>>(reference.downgrade())
    };
    let style = CSSStyleDeclaration::into_reference(CSSStyleDeclaration::new(parent_rule), env)?;
    self.style = Some(style.clone(env)?);
    Ok(style)
  }

  #[napi(setter)]
  pub fn set_style(&mut self, text: String, env: Env, reference: Reference<CSSFontFaceRule>) -> Result<()> {
    self.style(env, reference)?.set_css_text(text, env)?;
    Ok(())
  }
}

#[napi(js_name = "CSSFontPaletteValuesRule")]
struct CSSFontPaletteValuesRule {
  rule: CSSRule,
}

#[napi]
impl CSSFontPaletteValuesRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self { rule }
  }

  fn get(&self) -> Result<&FontPaletteValuesRule> {
    match self.rule.rule() {
      CssRule::FontPaletteValues(rule) => Ok(rule),
      _ => Err(napi::Error::new(
        napi::Status::GenericFailure,
        "Expected a @font-palette-values rule".into(),
      )),
    }
  }

  #[napi(getter)]
  pub fn name(&self) -> Result<&str> {
    Ok(self.get()?.name.0.as_ref())
  }

  #[napi(getter)]
  pub fn font_family(&self) -> Result<Option<String>> {
    Ok(self.get()?.properties.iter().rev().find_map(|property| match property {
      FontPaletteValuesProperty::FontFamily(family) => {
        Some(family.to_css_string(PrinterOptions::default()).unwrap())
      }
      _ => None,
    }))
  }

  #[napi(getter)]
  pub fn base_palette(&self) -> Result<Option<String>> {
    Ok(self.get()?.properties.iter().rev().find_map(|property| match property {
      FontPaletteValuesProperty::BasePalette(base_palette) => {
        Some(base_palette.to_css_string(PrinterOptions::default()).unwrap())
      }
      _ => None,
    }))
  }

  #[napi(getter)]
  pub fn override_colors(&self) -> Result<Option<String>> {
    Ok(self.get()?.properties.iter().rev().find_map(|property| match property {
      FontPaletteValuesProperty::OverrideColors(override_colors) => {
        Some(override_colors.to_css_string(PrinterOptions::default()).unwrap())
      }
      _ => None,
    }))
  }
}

#[napi(js_name = "CSSCounterStyleRule")]
struct CSSCounterStyleRule {
  rule: CSSRule,
  strings: Vec<String>,
}

#[napi]
impl CSSCounterStyleRule {
  #[napi(constructor)]
  pub fn constructor() {
    unreachable!()
  }

  fn new(rule: CSSRule) -> Self {
    Self {
      rule,
      strings: Vec::new(),
    }
  }

  fn add_string(&mut self, string: String) -> &'static str {
    let res = extend_lifetime(&string);
    self.strings.push(string);
    res
  }

  fn get(&self) -> Result<&CounterStyleRule> {
    match self.rule.rule() {
      CssRule::CounterStyle(rule) => Ok(rule),
      rule => {
        println!("{:?}", rule);
        Err(napi::Error::new(
          napi::Status::GenericFailure,
          "Expected a @counter-style rule".into(),
        ))
      }
    }
  }

  fn get_mut(&mut self) -> Result<&mut CounterStyleRule<'static>> {
    match self.rule.rule_mut() {
      CssRule::CounterStyle(rule) => Ok(rule),
      _ => Err(napi::Error::new(
        napi::Status::GenericFailure,
        "Expected a @counter-style rule".into(),
      )),
    }
  }

  #[napi(getter)]
  pub fn name(&self) -> Result<&str> {
    // TODO(interop): Firefox serializes the name as an identifier (e.g. escaping), Chrome does not.
    Ok(self.get()?.name.0.as_ref())
  }

  #[napi(setter)]
  pub fn set_name(&mut self, name: String) -> Result<()> {
    let rule = self.get_mut()?;
    rule.name.0 = name.into();
    Ok(())
  }

  fn find<'a, F, B: 'a>(&'a self, f: F) -> Result<String>
  where
    F: Fn(&'a CounterStyleProperty) -> Option<&'a B>,
    B: ToCss,
  {
    Ok(
      self
        .get()?
        .declarations
        .iter()
        .rev()
        .find_map(|property| f(property).map(|value| value.to_css_string(PrinterOptions::default()).unwrap()))
        .unwrap_or("".into()),
    )
  }

  fn update<F>(&mut self, value: CounterStyleProperty<'static>, f: F) -> Result<()>
  where
    F: Fn(&CounterStyleProperty) -> bool,
  {
    let declarations = &mut self.get_mut()?.declarations;
    for property in declarations.iter_mut().rev() {
      if f(property) {
        *property = value;
        return Ok(());
      }
    }

    declarations.push(value);
    Ok(())
  }

  #[napi(getter)]
  pub fn system(&self) -> Result<String> {
    self.find(|property| match property {
      CounterStyleProperty::System(system) => Some(system),
      _ => None,
    })
  }

  #[napi(setter)]
  pub fn set_system(&mut self, value: String) -> Result<()> {
    let system = System::parse_string(self.add_string(value)).unwrap();
    self.update(CounterStyleProperty::System(system), |property| {
      matches!(property, CounterStyleProperty::System(..))
    })
  }
}

fn extend_lifetime<T: ?Sized>(string: &T) -> &'static T {
  unsafe { std::mem::transmute::<&T, &'static T>(string) }
}

fn extend_lifetime_mut<T: ?Sized>(string: &mut T) -> &'static mut T {
  unsafe { std::mem::transmute::<&mut T, &'static mut T>(string) }
}
