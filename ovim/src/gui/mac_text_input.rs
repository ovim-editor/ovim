//! Keep macOS text substitutions out of Ovim's text fields.
//!
//! WebKit applies the system's typing substitutions to every web text field:
//! two spaces become ". ", quotes and dashes turn typographic, and text
//! replacements expand. Each corrupts code and shell commands typed into the
//! AI chat, diff comments or walkthrough questions. The `autocorrect`,
//! `spellcheck` and `autocapitalize` attributes do not reach these, so the
//! settings are turned off for this process.
//!
//! They go into the argument domain: it outranks the user's global setting
//! and is never written to disk, so other apps and the user's preferences
//! stay as they are.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{
    NSArgumentDomain, NSArray, NSMutableDictionary, NSNumber, NSString, NSUserDefaults,
};

/// AppKit user defaults that make WebKit rewrite typed text.
const SUBSTITUTION_DEFAULTS: [&str; 6] = [
    "NSAutomaticPeriodSubstitutionEnabled",
    "NSAutomaticQuoteSubstitutionEnabled",
    "NSAutomaticDashSubstitutionEnabled",
    "NSAutomaticTextReplacementEnabled",
    "NSAutomaticSpellingCorrectionEnabled",
    "NSAutomaticCapitalizationEnabled",
];

/// Whether `domain` is a volatile domain. `volatileDomainNames` returns nil,
/// not an empty array, when there is none, which the generated binding
/// cannot represent.
fn has_volatile_domain(defaults: &NSUserDefaults, domain: &NSString) -> bool {
    // SAFETY: the method takes no arguments and returns an NSArray of
    // NSString or nil.
    let names: Option<Retained<NSArray<NSString>>> =
        unsafe { msg_send![defaults, volatileDomainNames] };
    names.is_some_and(|names| names.containsObject(domain))
}

/// Turns off typing substitutions for this process. Call before any window
/// is created.
pub fn disable_text_substitutions() {
    let defaults = NSUserDefaults::standardUserDefaults();
    // SAFETY: a constant NSString exported by Foundation.
    let domain = unsafe { NSArgumentDomain };
    let entries = NSMutableDictionary::<NSString, AnyObject>::new();
    // Launch arguments of the form `-Key value` live in this domain too.
    // Setting an existing volatile domain raises, so it is merged and
    // replaced.
    if has_volatile_domain(&defaults, domain) {
        entries.addEntriesFromDictionary(&defaults.volatileDomainForName(domain));
        defaults.removeVolatileDomainForName(domain);
    }
    let off = NSNumber::numberWithBool(false);
    for key in SUBSTITUTION_DEFAULTS {
        entries.insert(&*NSString::from_str(key), &off);
    }
    // SAFETY: every key is an NSString and every value a property-list object.
    unsafe { defaults.setVolatileDomain_forName(&entries, domain) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_substitutions_off(defaults: &NSUserDefaults) {
        for key in SUBSTITUTION_DEFAULTS {
            assert!(
                !defaults.boolForKey(&NSString::from_str(key)),
                "{key} is still on"
            );
        }
    }

    // One test, because user defaults are shared by the whole process.
    #[test]
    fn substitutions_are_off_with_or_without_launch_arguments() {
        let defaults = NSUserDefaults::standardUserDefaults();
        // SAFETY: a constant NSString exported by Foundation.
        let domain = unsafe { NSArgumentDomain };
        if has_volatile_domain(&defaults, domain) {
            defaults.removeVolatileDomainForName(domain);
        }

        // An app launched without `-Key value` arguments has no such domain.
        disable_text_substitutions();
        assert_substitutions_off(&defaults);

        let argument = NSString::from_str("OvimTestLaunchArgument");
        let existing = NSMutableDictionary::<NSString, AnyObject>::new();
        existing.insert(&*argument, &*NSString::from_str("kept"));
        defaults.removeVolatileDomainForName(domain);
        // SAFETY: NSString keys and values.
        unsafe { defaults.setVolatileDomain_forName(&existing, domain) };

        disable_text_substitutions();
        assert_substitutions_off(&defaults);
        assert_eq!(
            defaults
                .stringForKey(&argument)
                .map(|value| value.to_string()),
            Some("kept".to_string())
        );
    }
}
