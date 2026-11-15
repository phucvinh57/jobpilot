//! Where a posting hires, in code: a small gazetteer of countries and regions, and the clear limit
//! phrases of a posting ("authorized to work in the US", "must be based in Canada", "EU only").
//! `score::hard_filter` uses it against `Preferences.work_from`, the countries the user may legally
//! work from.
//!
//! ```text
//! places_in("Remote - US or Canada")                 // [Country("United States"), Country("Canada")]
//! covers(&Place::Region("APAC"), &["Vietnam".into()])  // true: Vietnam is in APAC
//! restriction_in_jd("You must be authorized to work in the U.S.")  // [Country("United States")]
//! location_is_not_a_limit("Remote (outside US)")       // true: the location check is skipped
//! ```
//!
//! - Names match as whole words, longest first, and a match consumes its text: "Latin America" is
//!   the region, not the country "America" inside it.
//! - Short codes that are also English words or common abbreviations (`US`, `UK`, `VN`, `EU`,
//!   `SEA`) match in upper case only, so "join us" names no place.
//! - Only a few clear phrases count as a limit; a posting that merely names a country ("offices in
//!   the US and Vietnam", "a US-based company") limits nothing. A clause about time zones or
//!   working hours is not about where the hire may live, so it is left out. "<place> only" counts
//!   only in a sentence about hiring ("This role is US only", not "Our app is available in the US
//!   only") and without a negation before it ("not Bay Area only").
//! - A work_from entry the gazetteer does not know is covered by every region (it may be in it)
//!   and by a country of its own name only: only a clear mismatch may dismiss a posting.

use std::sync::OnceLock;

use regex::Regex;

/// A place the gazetteer knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// "Worldwide", "Anywhere", "Global": every country, also ones the gazetteer does not know.
    Anywhere,
    /// A country, by its name in [`COUNTRIES`].
    Country(&'static str),
    /// A region, by its name in [`REGIONS`].
    Region(&'static str),
}

impl Place {
    /// The place's name, as shown to the user.
    pub fn name(&self) -> &'static str {
        match self {
            Place::Anywhere => "anywhere",
            Place::Country(name) | Place::Region(name) => name,
        }
    }

    /// The countries the place stands for; `None` for [`Place::Anywhere`] (all of them).
    fn members(&self) -> Option<Vec<&'static str>> {
        match self {
            Place::Anywhere => None,
            Place::Country(name) => Some(vec![name]),
            Place::Region(name) => Some(
                REGIONS.iter().find(|r| r.name == *name).map(|r| r.members.iter().flat_map(|g| g.iter().copied()).collect()).unwrap_or_default(),
            ),
        }
    }
}

/// A country or region: its name, aliases matched in any case, and codes matched in upper case only.
struct Entry {
    name: &'static str,
    aliases: &'static [&'static str],
    upper: &'static [&'static str],
}

struct RegionEntry {
    name: &'static str,
    aliases: &'static [&'static str],
    upper: &'static [&'static str],
    /// Country names from [`COUNTRIES`], in groups (a country may be in more than one).
    members: &'static [&'static [&'static str]],
}

const fn country(name: &'static str, aliases: &'static [&'static str], upper: &'static [&'static str]) -> Entry {
    Entry { name, aliases, upper }
}

/// Countries, with their common spellings and a few big cities (a location like "London" or
/// "Hanoi" says the country). Ambiguous names (Georgia, Jordan, Washington) are left out.
const COUNTRIES: &[Entry] = &[
    country(
        "United States",
        &[
            "united states", "united states of america", "usa", "u.s.a.", "u.s.a", "u.s.", "u.s", "america",
            "new york", "nyc", "san francisco", "bay area", "silicon valley", "seattle", "austin", "boston", "chicago",
            "los angeles", "denver", "atlanta", "miami", "california", "texas", "florida", "massachusetts", "colorado",
            "illinois", "new jersey", "new mexico", "new england",
        ],
        &["US"],
    ),
    country(
        "United Kingdom",
        &["united kingdom", "u.k.", "great britain", "britain", "england", "scotland", "wales", "london", "manchester", "edinburgh"],
        &["UK", "GB"],
    ),
    country("Canada", &["canada", "toronto", "vancouver", "montreal", "ottawa", "calgary"], &[]),
    country(
        "Vietnam",
        &["vietnam", "viet nam", "việt nam", "ho chi minh city", "ho chi minh", "hcmc", "saigon", "hanoi", "ha noi", "hà nội", "da nang", "danang", "đà nẵng"],
        &["VN", "HCM"],
    ),
    country("Singapore", &["singapore"], &["SG"]),
    country("Australia", &["australia", "sydney", "melbourne", "brisbane", "perth", "new south wales"], &[]),
    country("New Zealand", &["new zealand", "auckland", "wellington"], &["NZ"]),
    country("India", &["india", "bangalore", "bengaluru", "mumbai", "new delhi", "delhi", "hyderabad", "pune", "chennai"], &[]),
    country("Pakistan", &["pakistan", "karachi", "lahore"], &[]),
    country("Bangladesh", &["bangladesh", "dhaka"], &[]),
    country("Sri Lanka", &["sri lanka", "colombo"], &[]),
    country("Nepal", &["nepal", "kathmandu"], &[]),
    country("Japan", &["japan", "tokyo", "osaka"], &[]),
    country("South Korea", &["south korea", "korea", "seoul"], &[]),
    country("China", &["china", "mainland china", "beijing", "shanghai", "shenzhen"], &[]),
    country("Hong Kong", &["hong kong"], &["HK"]),
    country("Taiwan", &["taiwan", "taipei"], &[]),
    country("Philippines", &["philippines", "manila"], &[]),
    country("Indonesia", &["indonesia", "jakarta", "bali"], &[]),
    country("Thailand", &["thailand", "bangkok"], &[]),
    country("Malaysia", &["malaysia", "kuala lumpur"], &[]),
    country("Cambodia", &["cambodia", "phnom penh"], &[]),
    country("Laos", &["laos", "vientiane"], &[]),
    country("Myanmar", &["myanmar", "burma", "yangon"], &[]),
    country("Brunei", &["brunei"], &[]),
    country("Germany", &["germany", "deutschland", "berlin", "munich", "münchen", "hamburg", "frankfurt"], &[]),
    country("France", &["france", "paris"], &[]),
    country("Netherlands", &["netherlands", "the netherlands", "holland", "amsterdam", "rotterdam"], &[]),
    country("Belgium", &["belgium", "brussels"], &[]),
    country("Spain", &["spain", "madrid", "barcelona"], &[]),
    country("Portugal", &["portugal", "lisbon", "porto"], &[]),
    country("Italy", &["italy", "milan", "rome"], &[]),
    country("Poland", &["poland", "warsaw", "krakow", "kraków"], &[]),
    country("Ireland", &["ireland", "dublin"], &[]),
    country("Sweden", &["sweden", "stockholm"], &[]),
    country("Denmark", &["denmark", "copenhagen"], &[]),
    country("Norway", &["norway", "oslo"], &[]),
    country("Finland", &["finland", "helsinki"], &[]),
    country("Switzerland", &["switzerland", "zurich", "zürich", "geneva"], &[]),
    country("Austria", &["austria", "vienna"], &[]),
    country("Czech Republic", &["czech republic", "czechia", "prague"], &[]),
    country("Romania", &["romania", "bucharest"], &[]),
    country("Ukraine", &["ukraine", "kyiv", "kiev"], &[]),
    country("Greece", &["greece", "athens"], &[]),
    country("Estonia", &["estonia", "tallinn"], &[]),
    country("Latvia", &["latvia", "riga"], &[]),
    country("Lithuania", &["lithuania", "vilnius"], &[]),
    country("Hungary", &["hungary", "budapest"], &[]),
    country("Bulgaria", &["bulgaria"], &[]),
    country("Croatia", &["croatia", "zagreb"], &[]),
    country("Slovakia", &["slovakia", "bratislava"], &[]),
    country("Slovenia", &["slovenia", "ljubljana"], &[]),
    country("Luxembourg", &["luxembourg"], &[]),
    country("Cyprus", &["cyprus", "limassol", "nicosia"], &[]),
    country("Malta", &["malta", "valletta"], &[]),
    country("Brazil", &["brazil", "brasil", "são paulo", "sao paulo", "rio de janeiro"], &[]),
    country("Mexico", &["mexico", "méxico", "mexico city"], &[]),
    country("Argentina", &["argentina", "buenos aires"], &[]),
    country("Colombia", &["colombia", "bogota", "bogotá", "medellin", "medellín"], &[]),
    country("Chile", &["chile", "santiago"], &[]),
    country("Peru", &["peru", "lima"], &[]),
    country("Uruguay", &["uruguay", "montevideo"], &[]),
    country("Costa Rica", &["costa rica"], &[]),
    country("Israel", &["israel", "tel aviv"], &[]),
    country("United Arab Emirates", &["united arab emirates", "uae", "dubai", "abu dhabi"], &[]),
    country("Saudi Arabia", &["saudi arabia", "riyadh"], &[]),
    country("Qatar", &["qatar", "doha"], &[]),
    country("Turkey", &["turkey", "türkiye", "istanbul"], &[]),
    country("Egypt", &["egypt", "cairo"], &[]),
    country("South Africa", &["south africa", "cape town", "johannesburg"], &[]),
    country("Nigeria", &["nigeria", "lagos"], &[]),
    country("Kenya", &["kenya", "nairobi"], &[]),
    country("Ghana", &["ghana", "accra"], &[]),
    country("Ethiopia", &["ethiopia", "addis ababa"], &[]),
    country("Morocco", &["morocco", "casablanca", "rabat"], &[]),
];

const SOUTHEAST_ASIA: &[&str] =
    &["Vietnam", "Singapore", "Philippines", "Indonesia", "Thailand", "Malaysia", "Cambodia", "Laos", "Myanmar", "Brunei"];
const SOUTH_ASIA: &[&str] = &["India", "Pakistan", "Bangladesh", "Sri Lanka", "Nepal"];
const EAST_ASIA: &[&str] = &["Japan", "South Korea", "China", "Hong Kong", "Taiwan"];
const OCEANIA: &[&str] = &["Australia", "New Zealand"];
/// The 27 member states.
const EU: &[&str] = &[
    "Austria", "Belgium", "Bulgaria", "Croatia", "Cyprus", "Czech Republic", "Denmark", "Estonia", "Finland", "France",
    "Germany", "Greece", "Hungary", "Ireland", "Italy", "Latvia", "Lithuania", "Luxembourg", "Malta", "Netherlands",
    "Poland", "Portugal", "Romania", "Slovakia", "Slovenia", "Spain", "Sweden",
];
/// Europe outside the EU.
const REST_OF_EUROPE: &[&str] = &["United Kingdom", "Norway", "Switzerland", "Ukraine"];
const MIDDLE_EAST: &[&str] = &["Israel", "United Arab Emirates", "Saudi Arabia", "Qatar", "Turkey", "Egypt"];
const NORTH_AFRICA: &[&str] = &["Egypt", "Morocco"];
const AFRICA: &[&str] = &["South Africa", "Nigeria", "Kenya", "Ghana", "Ethiopia", "Egypt", "Morocco"];
const LATAM: &[&str] = &["Mexico", "Brazil", "Argentina", "Colombia", "Chile", "Peru", "Uruguay", "Costa Rica"];
const NORTH_AMERICA: &[&str] = &["United States", "Canada", "Mexico"];

const fn region(
    name: &'static str,
    aliases: &'static [&'static str],
    upper: &'static [&'static str],
    members: &'static [&'static [&'static str]],
) -> RegionEntry {
    RegionEntry { name, aliases, upper, members }
}

const REGIONS: &[RegionEntry] = &[
    region("Southeast Asia", &["southeast asia", "south-east asia", "south east asia"], &["SEA"], &[SOUTHEAST_ASIA]),
    region("APAC", &["apac", "asia-pacific", "asia pacific", "asia/pacific", "asia & pacific"], &[], &[SOUTHEAST_ASIA, SOUTH_ASIA, EAST_ASIA, OCEANIA]),
    region("Asia", &["asia"], &[], &[SOUTHEAST_ASIA, SOUTH_ASIA, EAST_ASIA]),
    region("EU", &["european union"], &["EU"], &[EU]),
    region("Europe", &["europe"], &[], &[EU, REST_OF_EUROPE]),
    region("Middle East", &["middle east"], &[], &[MIDDLE_EAST]),
    region("MENA", &["mena"], &[], &[MIDDLE_EAST, NORTH_AFRICA]),
    region("Africa", &["africa"], &[], &[AFRICA]),
    region("EMEA", &["emea"], &[], &[EU, REST_OF_EUROPE, MIDDLE_EAST, AFRICA]),
    region("LATAM", &["latam", "latin america", "south america", "central america"], &[], &[LATAM]),
    region("North America", &["north america"], &[], &[NORTH_AMERICA]),
    region("Americas", &["americas", "the americas"], &[], &[NORTH_AMERICA, LATAM]),
];

/// Names that mean every country.
const ANYWHERE: &[&str] = &["worldwide", "world-wide", "world wide", "anywhere", "anywhere in the world", "global", "globally"];

/// One name to look for.
struct Pattern {
    /// Lower case unless `exact`.
    text: String,
    /// Match the case as written (upper-case codes).
    exact: bool,
    place: Place,
}

/// Every name of every place, longest first.
fn patterns() -> &'static [Pattern] {
    static PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let mut out = Vec::new();
        let mut add = |aliases: &[&str], upper: &[&str], place: Place| {
            out.extend(aliases.iter().map(|a| Pattern { text: a.to_lowercase(), exact: false, place }));
            out.extend(upper.iter().map(|u| Pattern { text: (*u).to_string(), exact: true, place }));
        };
        for c in COUNTRIES {
            add(c.aliases, c.upper, Place::Country(c.name));
        }
        for r in REGIONS {
            add(r.aliases, r.upper, Place::Region(r.name));
        }
        add(ANYWHERE, &[], Place::Anywhere);
        out.sort_by_key(|p| std::cmp::Reverse(p.text.len()));
        out
    })
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Every place named in `text`, with its byte range, in order of appearance.
fn find_places(text: &str) -> Vec<(usize, usize, Place)> {
    // ASCII lower case keeps every byte offset, so a range found in it is a range of `text`.
    let lower = text.to_ascii_lowercase();
    let mut found: Vec<(usize, usize, Place)> = Vec::new();
    for p in patterns() {
        let hay = if p.exact { text } else { lower.as_str() };
        for (start, m) in hay.match_indices(p.text.as_str()) {
            let end = start + m.len();
            let before_ok = hay[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
            let next = hay[end..].chars().next();
            // A word boundary after it, and an upper-case code is not a currency ("US$").
            let after_ok = next.is_none_or(|c| !is_word_char(c) && !(p.exact && c == '$'));
            let free = found.iter().all(|(s, e, _)| end <= *s || start >= *e);
            if before_ok && after_ok && free {
                found.push((start, end, p.place));
            }
        }
    }
    found.sort_by_key(|(s, _, _)| *s);
    found
}

fn dedup(places: impl IntoIterator<Item = Place>) -> Vec<Place> {
    let mut out: Vec<Place> = Vec::new();
    for p in places {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// The places `text` names, in order, each once.
pub fn places_in(text: &str) -> Vec<Place> {
    dedup(find_places(text).into_iter().map(|(_, _, p)| p))
}

/// What a user-typed `work_from` entry stands for: the place it is a name of (in any case), else
/// the places it mentions ("Hanoi, Vietnam").
fn resolve(entry: &str) -> Vec<Place> {
    let entry = entry.trim();
    if let Some(p) = patterns().iter().find(|p| p.text.eq_ignore_ascii_case(entry) || p.text == entry.to_lowercase()) {
        return vec![p.place];
    }
    places_in(entry)
}

fn overlap(a: &Place, b: &Place) -> bool {
    match (a.members(), b.members()) {
        (None, _) | (_, None) => true,
        (Some(a), Some(b)) => a.iter().any(|m| b.contains(m)),
    }
}

/// Whether someone who may work from the countries `work_from` (user-typed names) can take a job
/// in `place`: a country they name, a region one of them is in, or anywhere. For an entry the
/// gazetteer does not know, any region counts (it may be in it: only a clear mismatch may dismiss
/// a posting), and a country only by the same name. No `work_from` is no limit.
pub fn covers(place: &Place, work_from: &[String]) -> bool {
    if *place == Place::Anywhere || work_from.iter().all(|w| w.trim().is_empty()) {
        return true;
    }
    work_from.iter().filter(|w| !w.trim().is_empty()).any(|w| {
        let resolved = resolve(w);
        if resolved.is_empty() {
            matches!(place, Place::Region(_)) || place.name().eq_ignore_ascii_case(w.trim())
        } else {
            resolved.iter().any(|r| overlap(place, r))
        }
    })
}

// ---------------------------------------------------------------- limits stated in a posting

/// Longest clause read after a limit phrase.
const CLAUSE_CHARS: usize = 200;
/// How far before a limit phrase a negation or sponsorship word cancels it.
const LOOKBEHIND_CHARS: usize = 60;

/// Phrases after which the rest of the sentence says where the hire may be.
fn clause_starts() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:(?:authori[sz]ed|eligible|legally\s+able|legally\s+permitted|permitted)\s+to\s+work\s+(?:in|from|within)|must\s+(?:currently\s+)?(?:reside|live|be\s+located|be\s+based|be\s+a\s+resident)\s+(?:in|within)|(?:only|exclusively)\s+(?:hiring|recruiting|accepting\s+(?:candidates|applicants|applications)|open\s+to\s+(?:candidates|applicants))\s+(?:in|from|based\s+in|located\s+in|who\s+live\s+in|residing\s+in))\b",
        )
        .expect("static regex")
    })
}

/// "<place> only" / "<place>-only".
fn only_word() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(?:\s*-\s*|\s+)only\b").expect("static regex"))
}

/// "<place>-based candidates only", "<place> residents only".
fn people_only() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?:\s*-\s*based|\s+based)?\s+(?:candidates|applicants|applications|residents|citizens|talent|individuals)\s+only\b")
            .expect("static regex")
    })
}

/// Words that make a limit phrase say the opposite ("no need to be authorized to work in the US",
/// "we sponsor those not authorized to work in ...").
fn negated() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:not|no|never|without|sponsors?|sponsorship)\b|n't\b").expect("static regex"))
}

/// A clause about time zones or hours, not about where the hire lives.
fn about_hours() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:time\s*zones?|timezones?|tz|hours|overlap)\b").expect("static regex"))
}

/// The start of the sentence that holds `at`.
fn sentence_start(text: &str, at: usize) -> usize {
    text[..at].rfind(['\n', '!', '?', ';']).map_or(0, |i| i + 1).max(text[..at].rfind(". ").map_or(0, |i| i + 2))
}

/// The end of the sentence that starts at `from`: a line break, `!`, `?`, `;`, or a full stop
/// followed by the end or by a space and a capital ("U.S. or Canada" goes on, "U.S. We offer" ends).
fn sentence_end(text: &str, from: usize) -> usize {
    let rest = &text[from..];
    for (i, c) in rest.char_indices() {
        match c {
            '\n' | '!' | '?' | ';' => return from + i,
            '.' => {
                let after = &rest[i + 1..];
                let trimmed = after.trim_start();
                let spaced = trimmed.len() < after.len();
                if after.is_empty() || (spaced && trimmed.chars().next().is_none_or(char::is_uppercase)) {
                    return from + i;
                }
            }
            _ => {}
        }
    }
    text.len()
}

/// Cut `s` to at most `max` bytes, on a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// The last up to `max` bytes of `s`, on a character boundary.
fn tail(s: &str, max: usize) -> &str {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

/// Words that make a "<place> only" say the opposite ("We hire worldwide, not Bay Area only").
fn negated_only() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:not|no|never|without)\b|n't\b").expect("static regex"))
}

/// Words that make a sentence about who is hired, so its "<place> only" is a limit ("This role is
/// US only"), unlike "Our app is available in the US only".
fn about_hiring() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:hire[sd]?|hiring|candidates?|applicants?|based|residents?|residing|located|location|work|remote|role|position|eligible)\b",
        )
        .expect("static regex")
    })
}

/// A word that leaves a place out ("outside US", "Non-US", "ex-US", "excluding US").
fn excluding() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:outside|non|ex|excluding|except|not)\b").expect("static regex"))
}

/// Whether a posting's location names its places without saying the hire must be there: a word
/// that leaves a place out before it ("Remote (outside US)", "Remote - Non-US", "Remote, ex-US")
/// or a time zone ("Remote - US time zones", "Remote (EST hours)", "US hours").
pub(crate) fn location_is_not_a_limit(location: &str) -> bool {
    about_hours().is_match(location) || find_places(location).iter().any(|(start, _, _)| excluding().is_match(&location[..*start]))
}

/// The place named right before `at` in `text` ("Remote, US" -> US), with where its name starts.
fn place_just_before(text: &str, at: usize) -> Option<(usize, Place)> {
    let window = tail(&text[..at], 60);
    let offset = at - window.len();
    let window = window.trim_end_matches(|c: char| c.is_whitespace() || c == '(' || c == '-');
    find_places(window).into_iter().rev().find(|(_, end, _)| *end == window.len()).map(|(start, _, p)| (offset + start, p))
}

/// A "<place> only" whose place starts at `place_at` and whose phrase ends at `end` limits the
/// hire unless a negation comes before the place in its sentence; a bare "only" (not
/// "candidates only") also needs a word about hiring in the sentence.
fn only_limits(jd: &str, place_at: usize, end: usize, bare: bool) -> bool {
    let start = sentence_start(jd, place_at);
    if negated_only().is_match(tail(&jd[start..place_at], LOOKBEHIND_CHARS)) {
        return false;
    }
    !bare || about_hiring().is_match(&jd[start..sentence_end(jd, end)])
}

/// The limits a posting states about where the hire may be: the text of its limit clauses (the
/// part after the limit phrase, to the end of its sentence) and the places named right before an
/// "only". Read once, asked twice ([`JdLimits::places`], [`JdLimits::mentions`]).
pub(crate) struct JdLimits<'a> {
    clauses: Vec<&'a str>,
    named: Vec<Place>,
}

impl JdLimits<'_> {
    /// The places the limits name, each once.
    pub(crate) fn places(&self) -> Vec<Place> {
        dedup(self.clauses.iter().flat_map(|c| places_in(c)).chain(self.named.iter().copied()))
    }

    /// Whether one of the limit clauses names a `work_from` entry by its own words (an entry the
    /// gazetteer may not know: "authorized to work in Togo or the US").
    pub(crate) fn mentions(&self, work_from: &[String]) -> bool {
        self.clauses.iter().any(|c| work_from.iter().any(|w| super::source::mentions(c, w)))
    }
}

/// Read the limits of `jd`: see [`restriction_in_jd`] for the phrases.
pub(crate) fn jd_limits(jd: &str) -> JdLimits<'_> {
    let mut clauses = Vec::new();
    for m in clause_starts().find_iter(jd) {
        let start = sentence_start(jd, m.start());
        let lead_in = tail(&jd[start..m.start()], LOOKBEHIND_CHARS);
        if negated().is_match(lead_in) {
            continue;
        }
        let clause = cut(&jd[m.end()..sentence_end(jd, m.end())], CLAUSE_CHARS);
        if !about_hours().is_match(clause) {
            clauses.push(clause);
        }
    }
    let mut named = Vec::new();
    for m in people_only().find_iter(jd) {
        if let Some((at, place)) = place_just_before(jd, m.start()) {
            if only_limits(jd, at, m.end(), false) {
                named.push(place);
            }
        }
    }
    for m in only_word().find_iter(jd) {
        let hyphen = m.as_str().contains('-');
        let after = jd[m.end()..].trim_start_matches([' ', '\t']);
        // "US only" ends a phrase; "the US only through partners" is not a limit. "US-only" always is.
        let ends = after.is_empty() || after.starts_with(|c: char| ")],.;:|/!\n\r".contains(c));
        if !(hyphen || ends) {
            continue;
        }
        if let Some((at, place)) = place_just_before(jd, m.start()) {
            if only_limits(jd, at, m.end(), true) {
                named.push(place);
            }
        }
    }
    JdLimits { clauses, named }
}

/// The places a posting limits the hire to, in clear phrases only: "authorized (eligible, legally
/// able, permitted) to work in ...", "must reside (live, be located, be based) in ...", "only hiring
/// in ...", "US-based candidates only", and "US only" / "US-only" in a sentence about hiring ("This
/// role is US only", not "Our app is available in the US only"). A limit phrase after "not",
/// "without" or "sponsor" does not count, and nor does a clause about time zones or hours.
pub fn restriction_in_jd(jd: &str) -> Vec<Place> {
    jd_limits(jd).places()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countries(names: &[&'static str]) -> Vec<Place> {
        names.iter().copied().map(Place::Country).collect()
    }

    fn vn() -> Vec<String> {
        vec!["Vietnam".into()]
    }

    #[test]
    fn every_region_member_is_a_known_country() {
        for r in REGIONS {
            for m in r.members.iter().flat_map(|g| g.iter()) {
                assert!(COUNTRIES.iter().any(|c| c.name == *m), "{} lists {m}", r.name);
            }
        }
    }

    #[test]
    fn the_eu_has_its_27_members_and_europe_and_emea_hold_them() {
        let eu = Place::Region("EU").members().unwrap();
        assert_eq!(eu.len(), 27);
        for name in ["Hungary", "Bulgaria", "Croatia", "Slovakia", "Slovenia", "Latvia", "Lithuania", "Luxembourg", "Cyprus", "Malta"] {
            assert!(eu.contains(&name), "{name}");
            assert_eq!(places_in(name), countries(&[name]));
            for region in ["Europe", "EMEA"] {
                assert!(covers(&Place::Region(region), &[name.to_string()]), "{name} in {region}");
            }
        }
        assert_eq!(places_in("Czechia"), countries(&["Czech Republic"]));
    }

    #[test]
    fn more_countries_and_their_regions_are_known() {
        let cases: [(&str, &[&str]); 9] = [
            ("Taiwan", &["Asia", "APAC"]),
            ("Pakistan", &["Asia", "APAC"]),
            ("Cambodia", &["Southeast Asia", "Asia", "APAC"]),
            ("Nigeria", &["Africa", "EMEA"]),
            ("Ghana", &["Africa", "EMEA"]),
            ("Morocco", &["Africa", "EMEA", "MENA"]),
            ("Qatar", &["Middle East", "MENA", "EMEA"]),
            ("Chile", &["LATAM", "Americas"]),
            ("Uruguay", &["LATAM", "Americas"]),
        ];
        for (country, regions) in cases {
            assert_eq!(places_in(&format!("Remote - {country}")), countries(&[country]));
            for region in regions {
                assert!(covers(&Place::Region(region), &[country.to_string()]), "{country} in {region}");
            }
            assert!(!covers(&Place::Region("North America"), &[country.to_string()]), "{country}");
        }
        for name in ["Sri Lanka", "Bangladesh", "Nepal", "Laos", "Myanmar", "Kenya", "Egypt", "South Africa", "Ethiopia", "Turkey", "Saudi Arabia", "Colombia", "Peru"] {
            assert_eq!(places_in(name), countries(&[name]), "{name}");
        }
        assert_eq!(places_in("Remote - MENA"), vec![Place::Region("MENA")]);
    }

    #[test]
    fn places_are_found_by_whole_words_and_aliases() {
        let us = Place::Country("United States");
        assert_eq!(places_in("Remote - US"), vec![us]);
        assert_eq!(places_in("Remote (USA)"), vec![us]);
        assert_eq!(places_in("Remote, U.S."), vec![us]);
        assert_eq!(places_in("New York, NY"), vec![us]);
        assert_eq!(places_in("Remote - US or Vietnam"), countries(&["United States", "Vietnam"]));
        assert_eq!(places_in("Ho Chi Minh City"), countries(&["Vietnam"]));
        assert_eq!(places_in("Remote - Viet Nam / HCMC"), countries(&["Vietnam"]));
        assert_eq!(places_in("London, UK"), countries(&["United Kingdom"]));
        assert_eq!(places_in("Remote (APAC)"), vec![Place::Region("APAC")]);
        assert_eq!(places_in("Remote - Worldwide"), vec![Place::Anywhere]);
        assert_eq!(places_in("Remote"), vec![]);
        assert_eq!(places_in(""), vec![]);
        assert_eq!(places_in("Indiana"), vec![], "India is a whole word only");
    }

    #[test]
    fn short_codes_match_in_upper_case_only() {
        assert_eq!(places_in("Join us and build with us"), vec![]);
        assert_eq!(places_in("sea views, uk-ish"), vec![]);
        assert_eq!(places_in("Remote, EU"), vec![Place::Region("EU")]);
        assert_eq!(places_in("Remote, SEA"), vec![Place::Region("Southeast Asia")]);
        assert_eq!(places_in("Pay: US$120k"), vec![], "a currency is not a place");
        assert_eq!(places_in("USD 120k"), vec![]);
    }

    #[test]
    fn longer_names_are_taken_first() {
        assert_eq!(places_in("Remote - Latin America"), vec![Place::Region("LATAM")]);
        assert_eq!(places_in("North America"), vec![Place::Region("North America")]);
        assert_eq!(places_in("Americas"), vec![Place::Region("Americas")]);
        assert_eq!(places_in("Southeast Asia"), vec![Place::Region("Southeast Asia")]);
        assert_eq!(places_in("Asia-Pacific"), vec![Place::Region("APAC")]);
        assert_eq!(places_in("South Africa"), countries(&["South Africa"]));
        assert_eq!(places_in("South Korea"), countries(&["South Korea"]));
        assert_eq!(places_in("Albuquerque, New Mexico"), countries(&["United States"]));
    }

    #[test]
    fn work_from_covers_its_countries_their_regions_and_anywhere() {
        let wf = vn();
        assert!(covers(&Place::Country("Vietnam"), &wf));
        assert!(covers(&Place::Region("APAC"), &wf));
        assert!(covers(&Place::Region("Asia"), &wf));
        assert!(covers(&Place::Region("Southeast Asia"), &wf));
        assert!(covers(&Place::Anywhere, &wf));
        assert!(!covers(&Place::Country("United States"), &wf));
        assert!(!covers(&Place::Region("EMEA"), &wf));
        assert!(!covers(&Place::Region("LATAM"), &wf));
        // User-typed spellings resolve like posting text, in any case.
        assert!(covers(&Place::Country("Vietnam"), &["viet nam".into()]));
        assert!(covers(&Place::Country("Vietnam"), &["Hanoi, Vietnam".into()]));
        assert!(covers(&Place::Country("United States"), &["us".into()]));
        assert!(covers(&Place::Region("Europe"), &["Germany".into()]));
        // An entry the gazetteer does not know: a country only by its name, any region (it may be
        // in it), anywhere.
        let atlantis = vec!["Atlantis".to_string()];
        assert!(!covers(&Place::Country("United States"), &atlantis));
        assert!(covers(&Place::Region("EMEA"), &atlantis));
        assert!(covers(&Place::Region("LATAM"), &atlantis));
        assert!(covers(&Place::Anywhere, &atlantis));
        assert!(covers(&Place::Country("United States"), &[]), "no work_from, no limit");
    }

    #[test]
    fn clear_limit_phrases_are_restrictions() {
        let us = Place::Country("United States");
        assert_eq!(restriction_in_jd("You must be authorized to work in the United States."), vec![us]);
        assert_eq!(restriction_in_jd("Candidates must be legally authorised to work in the UK"), countries(&["United Kingdom"]));
        assert_eq!(restriction_in_jd("You must be eligible to work in the U.S. or Vietnam."), countries(&["United States", "Vietnam"]));
        assert_eq!(restriction_in_jd("Authorized to work in the US or Vietnam. We offer equity."), countries(&["United States", "Vietnam"]));
        assert_eq!(restriction_in_jd("You must reside in Canada."), countries(&["Canada"]));
        assert_eq!(restriction_in_jd("You must be based in the EU"), vec![Place::Region("EU")]);
        assert_eq!(restriction_in_jd("We are only hiring in Brazil and Mexico right now."), countries(&["Brazil", "Mexico"]));
        assert_eq!(restriction_in_jd("Remote (US only)"), vec![us]);
        assert_eq!(restriction_in_jd("This role is US-only and remote"), vec![us]);
        assert_eq!(restriction_in_jd("Location: USA only"), vec![us]);
        assert_eq!(restriction_in_jd("Open to US-based candidates only."), vec![us]);
        assert_eq!(restriction_in_jd("Canadian roles: Canada residents only"), countries(&["Canada"]));
    }

    #[test]
    fn naming_a_country_is_not_a_restriction() {
        for jd in [
            "We have offices in the US and Vietnam.",
            "US-based company with a remote team in Asia.",
            "We serve customers across the US only through partners",
            "You will work with us to build a better product. Join us!",
            "You must be able to work in US time zones.",
            "Must be located in a time zone that overlaps with US hours.",
            "You do not need to be authorized to work in the US; we sponsor visas.",
            "We sponsor candidates who are not authorized to work in the US.",
            "Pay: US$150k. Remote anywhere.",
        ] {
            assert_eq!(restriction_in_jd(jd), vec![], "{jd}");
        }
    }

    #[test]
    fn only_needs_a_sentence_about_hiring_and_no_negation() {
        let us = Place::Country("United States");
        for jd in [
            "Our app is currently available in the US only.",
            "We hire worldwide, not Bay Area only.",
            "We never hire US-based candidates only.",
            "Great product. Shipping to the UK only. We are remote-first.",
        ] {
            assert_eq!(restriction_in_jd(jd), vec![], "{jd}");
        }
        assert_eq!(restriction_in_jd("This role is US only."), vec![us]);
        assert_eq!(restriction_in_jd("Remote, US-based candidates only."), vec![us]);
        assert_eq!(restriction_in_jd("We ship worldwide. This position is UK only."), countries(&["United Kingdom"]));
    }

    #[test]
    fn a_location_that_leaves_a_place_out_or_names_a_time_zone_is_not_a_limit() {
        for loc in [
            "Remote (outside US)",
            "Remote - Non-US",
            "Remote, ex-US",
            "Remote - excluding US",
            "Remote, except the UK",
            "Remote - not US",
            "Remote - US time zones",
            "Remote (US timezone)",
            "Remote (EST hours)",
            "US hours",
            "Remote, US TZ",
            "Remote (4h overlap with UK)",
        ] {
            assert!(location_is_not_a_limit(loc), "{loc}");
        }
        for loc in ["Remote - US", "Remote (USA / Canada)", "Berlin, Germany", "Remote - Mexico", "Remote, Texas", "Remote"] {
            assert!(!location_is_not_a_limit(loc), "{loc}");
        }
    }

    #[test]
    fn a_limit_clause_can_name_a_country_the_gazetteer_does_not_know() {
        let jd = "You must be authorized to work in Togo or the US.";
        let limits = jd_limits(jd);
        assert_eq!(limits.places(), countries(&["United States"]));
        assert!(limits.mentions(&["Togo".into()]));
        assert!(!limits.mentions(&["Benin".into()]));
        assert_eq!(restriction_in_jd(jd), limits.places());
    }
}
