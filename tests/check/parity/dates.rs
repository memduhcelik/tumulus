//! Date filters compared with jq.

use super::*;

#[test]
fn dates_match_jq() {
    require_jq();
    let programs = [
        "0 | todate",
        "1 | todate",
        "1.9 | todate",
        "-0 | todate",
        "-1 | todate",
        "-1.9 | todate",
        "-0.1 | todate",
        "nan | todate",
        "86400 | todate",
        "1600000000 | todate",
        "1600000000 | todate | fromdate",
        "1e10 | todate",
        "1e16 | todate",
        "-1e16 | todate",
        "-315619200 | todate",
        "-2208988800 | todate",
        "[1970,0,1,0,0,0,4,0] | todate",
        "[2015,2,5,23,51,47,4,63] | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | gmtime",
        "1 | gmtime",
        "1.9 | gmtime",
        "-0 | gmtime",
        "-0.1 | gmtime",
        "-0.9 | gmtime",
        "-1 | gmtime",
        "-1.1 | gmtime",
        "-1.9 | gmtime",
        "nan | gmtime",
        "nan | gmtime | .[5] == null",
        "nan | gmtime | .[5] | length",
        "86400 | gmtime",
        "1425599507 | gmtime",
        "1425599507 | gmtime | .[0] == 2015",
        "1600000000.9 | gmtime",
        "1e16 | gmtime",
        "1e16 | gmtime | mktime",
        "-1e16 | gmtime",
        "9007199254740993 | gmtime | .[0]",
        "0 | gmtime | mktime",
        "1.9 | gmtime | mktime",
        "1600000000 | gmtime | mktime",
        "86400 | gmtime | mktime | todate",
        "[1970,0,1,0,0,0,4,0] | mktime",
        "[1970,0,1,0,0,0,0,0] | mktime",
        "[1970,0,1,0,0,0,4,99] | mktime",
        "[1970,0,1,0,0,60,4,0] | mktime",
        "[1970,0,1,0,0,61,4,0] | mktime",
        "[1970,12,1,0,0,0,4,0] | mktime",
        "[1970,0,0,0,0,0,4,0] | mktime",
        "[1970,0,1,24,0,0,4,0] | mktime",
        "[1970.9,0.9,1.9,0.9,0.9,0.9,4.9,0.9] | mktime",
        "[1970,0,1,0,0,1.9,4,0] | mktime",
        "[1970,0,1,0,0,nan,4,0] | mktime",
        "[1970,0,1,0,0,infinite,4,0] | mktime",
        "[1970,0,1,0,0,0,4,0,\"x\"] | mktime",
        "[1970,0,1,0,0,0,4,0,null] | mktime",
        "[infinite,0,1,0,0,0,4,0] | mktime",
        "\"1970-01-01T00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:01Z\" | fromdate",
        "\"2000-01-01T00:00:00Z\" | fromdate",
        "\"2000-02-29T00:00:00Z\" | fromdate",
        "\"2016-12-31T23:59:59Z\" | fromdate | todate",
        "\"1960-01-01T00:00:00Z\" | fromdate",
        "\"1900-01-01T00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:00Z \" | fromdate",
        "\"1970-01-01T00:00:00Z\\n\" | fromdate",
        "\"1970-01-01T00:00:00Z\\t\" | fromdate",
        "\"1970-01-01T00:00:00Z\\r\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u000b\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u000c\" | fromdate",
        "\"1970-01-01T00:00:00Z junk\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u00a0\" | fromdate",
        "0 | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | strftime(\"%Y %m %d %H %M %S\")",
        "0 | strftime(\"%Z\")",
        "0 | strftime(\"year %Y\")",
        "0 | strftime(\"年%Y月\")",
        "0 | strftime(\"hello\")",
        "0 | strftime(\"%Y\", \"%m\")",
        "(0, 86400) | strftime(\"%d\")",
        "1.9 | strftime(\"%S\")",
        "1.9 | gmtime | strftime(\"%S\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%S\")",
        "nan | strftime(\"%Y\")",
        "0 | strftime(empty)",
        "\"x\" | strftime(\"%Y\")?",
        "true | todate?",
        "(\"x\" | gmtime)?, 1",
        "{todate: 1}",
        "{todate}",
        "{gmtime: 1}",
        "{strftime: 1}",
        "[0, 1.9, 86400] | map(todate)",
        "0 | strftime (\"%Y\")",
        "strftime(\"%Y\")",
        "0 | gmtime | strftime(\"%Y-%m-%d\")",
        "2 | gmtime | .[0]",
        "[0] | .[0] |= todate",
        "infinite | todate",
        "-infinite | todate",
        "true | todate",
        "\"x\" | todate",
        "null | todate",
        "[1970,0,1,0,0,0] | todate",
        "infinite | gmtime",
        "-infinite | gmtime",
        "true | gmtime",
        "\"0\" | gmtime",
        "null | gmtime",
        "[0] | gmtime",
        "1e17 | gmtime",
        "1e18 | gmtime",
        "-1 | gmtime | mktime",
        "[1969,11,31,23,59,59,3,364] | mktime",
        "[1969,11,31,23,59,58,3,364] | mktime",
        "[1970,0,1,0,0] | mktime",
        "[1970,0,1,0,0,0] | mktime",
        "[1970,0,1,0,0,0,4] | mktime",
        "[1970,0,1,0,0,0,4,null] | mktime",
        "[1970,0,1,0,0,0,4,\"0\"] | mktime",
        "[true,0,1,0,0,0,4,0] | mktime",
        "true | mktime",
        "0 | mktime",
        "{} | mktime",
        "[] | mktime",
        "\"1970-01-01T00:00:00Zjunk\" | fromdate",
        "\"1970-01-01T00:00:00ZZ\" | fromdate",
        "\"1970-01-01T00:00:00\" | fromdate",
        "\"1970-01-01t00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:00.5Z\" | fromdate",
        "\"1970-01-01T00:00:00+00:00\" | fromdate",
        "\"bad\" | fromdate",
        "0 | fromdate",
        "true | fromdate",
        "null | fromdate",
        "[] | fromdate",
        "\"1969-12-31T23:59:59Z\" | fromdate",
        "\"1969-12-31T23:59:58Z\" | fromdate",
        "\"0001-01-01T00:00:00Z\" | fromdate",
        "\"1970\\n-01\" | fromdate",
        "\"say \\\"hi\\\"\" | fromdate",
        "true | strftime(\"%Y\")",
        "\"x\" | strftime(\"%Y\")",
        "null | strftime(\"%Y\")",
        "[1970,0,1,0,0,0] | strftime(\"%Y\")",
        "[true,0,1,0,0,0,4,0] | strftime(\"%Y\")",
        "[1970,0,1,0,0,0,4,0] | strftime(1)",
        "[1970,0,1] | strftime(1)",
        "[true,0,1,0,0,0,4,0] | strftime(1)",
        "infinite | strftime(\"%Y\")",
        "infinite | strftime(1)",
        "infinite | strftime(\"%Y\", \"%m\")",
        "0 | strftime(\"\")",
        "0 | strftime(\"%Y\", 1/0)",
        "0 | strftime(1/0)",
        "0 | strftime(1/0, \"%Y\")",
        "1, (\"x\" | todate)",
        "(\"x\" | todate), 1",
        "1, (\"x\" | gmtime)",
        "(\"no\" | fromdate), 1",
        "\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:01Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2000-02-29T12:30:45Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1900-03-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2000-03-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1969-12-31T23:59:59Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2020-07-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z \" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z junk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\n\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\tmore\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\r\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u000b\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u000c\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u00a0\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"\" | strptime(\"\")",
        "\" \" | strptime(\"\")",
        "\"1970\" | strptime(\"\")",
        "\"1970\" | strptime(\"%Y\")",
        "\"1970-01\" | strptime(\"%Y-%m\")",
        "\"70\" | strptime(\"%y\")",
        "\"69\" | strptime(\"%y\")",
        "\"68\" | strptime(\"%y\")",
        "\"00\" | strptime(\"%y\")",
        "\"99\" | strptime(\"%y\")",
        "\"Jan\" | strptime(\"%b\")",
        "\"Feb\" | strptime(\"%b\")",
        "\"Dec\" | strptime(\"%b\")",
        "\"jan\" | strptime(\"%b\")",
        "\"Thu\" | strptime(\"%a\")",
        "\" 1\" | strptime(\"%e\")",
        "\"1\" | strptime(\"%e\")",
        "\"10\" | strptime(\"%e\")",
        "\" 5\" | strptime(\"%e\")",
        "\"1970-01-01 001\" | strptime(\"%Y-%m-%d %j\")",
        "\"2000-02-29 060\" | strptime(\"%Y-%m-%d %j\")",
        "\"2000-03-01 061\" | strptime(\"%Y-%m-%d %j\")",
        "\"1970 extra\" | strptime(\"%Y\")",
        "\"1970 \\\"z\\\"\" | strptime(\"%Y\")",
        "\"1970\\n01\" | strptime(\"%Y %m\")",
        "0 | gmtime == (\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\"))",
        "0 | strftime(\"%Y-%m-%dT%H:%M:%SZ\") | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\") | mktime",
        "\"1970-01-01T00:00:00Z junk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\") | mktime",
        "\"1970\" | strptime(\"%Y\", \"%m\")",
        "\"1970-01\" | strptime(\"%Y-%m\", 1/0)",
        "\"\" | strptime(1/0, \"%Y\")",
        "1 | strptime(1/0)",
        "\"bad\" | strptime(\"%Y\")?",
        "(\"bad\" | strptime(\"%Y\")), 1",
        "1, (\"bad\" | strptime(\"%Y\"))",
        "\"1970\" | strptime (\"%Y\")",
        "1 | strptime(\"%Y\")",
        "\"x\" | strptime(1)",
        "1 | strptime(1)",
        "true | strptime(\"%Y\")",
        "null | strptime(\"%Y\")",
        "[] | strptime(\"%Y\")",
        "\"1970\" | strptime(null)",
        "\"1970\" | strptime(true)",
        "\"bad\" | strptime(\"%Y-%m-%d\")",
        "\"1970-01-01T00:00:00Zjunk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00ZZ\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"\" | strptime(\"%Y\")",
        "\"1970\" | strptime(\"%Y-%m-%d\")",
        "\"%Y\" | strptime(\"%Y\")",
        "\"1970\" | strptime(\"%Y extra\")",
        "0 | strftime(\"%j\")",
        "0 | strftime(\"%a\")",
        "0 | strftime(\"%b\")",
        "0 | strftime(\"%e\")",
        "0 | strftime(\"%y\")",
        "0 | strftime(\"%z\")",
        "0 | strftime(\"%Y %j %a %b %e %y\")",
        "86400 | strftime(\"%j %a %e\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%a %b %d %e %y %j\")",
        "[1970,0,5,0,0,0,4,4] | strftime(\"%e\")",
        "[1970,0,10,0,0,0,4,9] | strftime(\"%e\")",
        "1593561600 | strftime(\"%z %Z %a %j\")",
        "\"1970-01-01T00:00:00+0000\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
        "\"1970-01-01T00:00:00-0700\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
        "empty | now",
        "now > 0",
        "now > 1600000000",
        "1 | now > 1600000000",
        "true | now > 1600000000",
        "now | gmtime | length",
        "now | gmtime | .[0] > 2020",
        "1,2 | now > 0",
        "{now: 1}",
        "{now}",
        "{strptime: 1}",
        "{strptime}",
        "0 | localtime",
        "1 | localtime",
        "1.9 | localtime",
        "-0 | localtime",
        "-0.1 | localtime",
        "-0.9 | localtime",
        "-1 | localtime",
        "-1.1 | localtime",
        "-1.9 | localtime",
        "nan | localtime",
        "nan | localtime | .[5] == null",
        "nan | localtime | .[5] | length",
        "86400 | localtime",
        "1425599507 | localtime",
        "1600000000.9 | localtime",
        "1e16 | localtime",
        "1e16 | strflocaltime(\"%Y\")",
        "-1e16 | localtime",
        "-1e16 | strflocaltime(\"%Y\")",
        "9007199254740993 | localtime | .[0]",
        "9007199254740993 | strflocaltime(\"%Y\")",
        "0 | localtime | .[5]",
        "0 | gmtime == localtime",
        "1593561600 | localtime",
        "localtime",
        "0 | strflocaltime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | strflocaltime(\"%Y %m %d %H %M %S\")",
        "0 | strflocaltime(\"%Z\")",
        "0 | strflocaltime(\"%z\")",
        "0 | strflocaltime(\"%j\")",
        "0 | strflocaltime(\"%a\")",
        "0 | strflocaltime(\"%b\")",
        "0 | strflocaltime(\"%e\")",
        "0 | strflocaltime(\"%y\")",
        "0 | strflocaltime(\"%Y %m %d %H %M %S %Z %z %j %a %b %e %y\")",
        "0 | strflocaltime(\"%Y\", \"%m\")",
        "(0, 86400) | strflocaltime(\"%d\")",
        "1.9 | strflocaltime(\"%S\")",
        "1.9 | localtime | strflocaltime(\"%S\")",
        "[1970,0,1,0,0,1.9,4,0] | strflocaltime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "[1970,0,1,0,0,1.9,4,0] | strflocaltime(\"%S\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%Z %z\") == ([1970,0,1,0,0,0,4,0] | strflocaltime(\"%Z %z\"))",
        "0 | strflocaltime(empty)",
        "\"x\" | localtime?",
        "(\"x\" | localtime)?, 1",
        "{localtime: 1}",
        "{localtime}",
        "{strflocaltime: 1}",
        "{strflocaltime}",
        "[0, 1.9, 86400] | map(localtime)",
        "0 | strflocaltime (\"%Y\")",
        "strflocaltime(\"%Y\")",
        "0 | localtime | strflocaltime(\"%Y-%m-%d\")",
        "1593561600 | strflocaltime(\"%Y-%m-%d %H:%M:%S %Z %z %a %j\")",
        "1593561600 | localtime | strflocaltime(\"%Y-%m-%d %H:%M:%S\")",
        "86400 | strflocaltime(\"%j %a %e\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%a %b %d %e %y %j\")",
        "[1970,0,5,0,0,0,4,4] | strflocaltime(\"%e\")",
        "[1970,0,10,0,0,0,4,9] | strflocaltime(\"%e\")",
        "infinite | localtime",
        "-infinite | localtime",
        "true | localtime",
        "\"0\" | localtime",
        "null | localtime",
        "[0] | localtime",
        "1e17 | localtime",
        "1e18 | localtime",
        "true | strflocaltime(\"%Y\")",
        "\"x\" | strflocaltime(\"%Y\")",
        "null | strflocaltime(\"%Y\")",
        "[1970,0,1,0,0,0] | strflocaltime(\"%Y\")",
        "[true,0,1,0,0,0,4,0] | strflocaltime(\"%Y\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(1)",
        "[1970,0,1] | strflocaltime(1)",
        "[true,0,1,0,0,0,4,0] | strflocaltime(1)",
        "0 | strflocaltime(\"\")",
        "0 | strflocaltime(\"%Y\", 1/0)",
        "0 | strflocaltime(1/0)",
        "0 | strflocaltime(1/0, \"%Y\")",
        "1, (\"x\" | localtime)",
        "(\"x\" | localtime), 1",
        "empty | localtime",
    ];
    each(programs.len(), |index| {
        let program = programs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });
    let streams = [
        ("todate", "0\n1\n\"x\"\n"),
        ("gmtime", "0\n1\n\"x\"\n"),
        ("todate", "0\n\"x\"\n1\n"),
        ("fromdate", "\"1970-01-01T00:00:00Z\"\n\"bad\"\n"),
        ("gmtime", "0\ntrue\n1\n"),
        (
            "strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
            "\"1970-01-01T00:00:00Z\"\n\"bad\"\n",
        ),
        ("strptime(\"%Y\")", "\"bad\"\n\"1970\"\n"),
        ("now > 0", "1\ntrue\n"),
        ("localtime", "0\n1\n\"x\"\n"),
        ("localtime", "0\ntrue\n1\n"),
        ("strflocaltime(\"%Y\")", "0\n\"x\"\n1\n"),
        ("strflocaltime(\"%Y\")", "0\ntrue\n"),
    ];
    for (program, input) in streams {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
    for tz in [
        "UTC",
        "GMT",
        "America/Los_Angeles",
        "Asia/Kolkata",
        "Europe/Istanbul",
    ] {
        compare_tz("epoch zone", "0 | strftime(\"%Z\")", tz);
        compare_tz("summer zone", "1593561600 | strftime(\"%Z\")", tz);
        compare_tz(
            "utc fields",
            "1593561600 | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
            tz,
        );
        compare_tz("epoch offset", "0 | strftime(\"%z\")", tz);
        compare_tz("summer offset", "1593561600 | strftime(\"%z %a %b\")", tz);
        compare_tz(
            "parsed offset",
            "\"1970-01-01T00:00:00+0000\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
            tz,
        );
        compare_tz(
            "parsed negative offset",
            "\"1970-01-01T00:00:00-0700\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
            tz,
        );
        compare_tz("local epoch", "0 | localtime", tz);
        compare_tz("local summer", "1593561600 | localtime", tz);
        compare_tz("local fraction", "1.9 | localtime", tz);
        compare_tz("local before epoch", "-1.1 | localtime", tz);
        compare_tz(
            "local format epoch",
            "0 | strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z %a %j\")",
            tz,
        );
        compare_tz(
            "local format summer",
            "1593561600 | strflocaltime(\"%Y-%m-%d %H:%M:%S %Z %z %a %j\")",
            tz,
        );
        compare_tz(
            "local format letters",
            "1593561600 | strflocaltime(\"%Y %m %d %H %M %S %Z %z %j %a %b %e %y\")",
            tz,
        );
        compare_tz(
            "array local zone",
            "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Z %z\")",
            tz,
        );
        compare_tz("same instant", "0 | gmtime == localtime", tz);
        compare_tz(
            "epoch names",
            "0 | strftime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
            tz,
        );
        compare_tz(
            "local names",
            "0 | strflocaltime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
            tz,
        );
        compare_tz(
            "summer names",
            "1593561600 | strftime(\"%A %B %I %p %u %w %s %c\")",
            tz,
        );
        compare_tz(
            "local summer names",
            "1593561600 | strflocaltime(\"%A %B %I %p %u %w %s %c\")",
            tz,
        );
        compare_tz("parsed seconds", "\"0\" | strptime(\"%s\")", tz);
        compare_tz("parsed afternoon", "\"1 PM\" | strptime(\"%I %p\")", tz);
    }
}

#[test]
fn date_letters_match_jq() {
    require_jq();
    let programs = [
        "0 | strftime(\"%A\")",
        "0 | strftime(\"%B\")",
        "0 | strftime(\"%I\")",
        "0 | strftime(\"%p\")",
        "0 | strftime(\"%u\")",
        "0 | strftime(\"%w\")",
        "0 | strftime(\"%s\")",
        "0 | strftime(\"%D\")",
        "0 | strftime(\"%F\")",
        "0 | strftime(\"%T\")",
        "0 | strftime(\"%R\")",
        "0 | strftime(\"%c\")",
        "0 | strftime(\"%%\")",
        "0 | strflocaltime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
        "3600 | strftime(\"%I %p %R %T\")",
        "43200 | strftime(\"%I %p\")",
        "46800 | strftime(\"%I %p\")",
        "86399 | strftime(\"%I %p %H\")",
        "1.9 | strftime(\"%s %S\")",
        "1600000000 | strftime(\"%A %B %I %p %u %w %s %D %F\")",
        "1600000000 | strflocaltime(\"%A %B %I %p %u %w %s %c\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%A %a %B %b %u %w %j %s\")",
        "[1970,0,1,0,0,0,0,0] | strftime(\"%A %u %w\")",
        "[1970,0,4,0,0,0,0,3] | strftime(\"%A %u %w\")",
        "[1970,0,1,0,30,0,4,0] | strftime(\"%I %p %R %T\")",
        "[1970,0,1,12,0,0,4,0] | strftime(\"%I %p\")",
        "[1970,0,1,15,0,0,4,0] | strftime(\"%I %p %H\")",
        "[1970,6,1,0,0,0,3,181] | strftime(\"%A %B %u %w\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%s %S %c\")",
        "[2015,2,5,23,51,47,4,63] | strftime(\"%s\")",
        "[2015,2,5,23,51,47,4,63] | strflocaltime(\"%s\")",
        "0 | strftime(\"100%%\")",
        "0 | strftime(\"%Y%%\")",
        "0 | strftime(\"%%Y\")",
        "0 | strftime(\"%A%B%I%p%u%w%s%D%F%T%R%c%%\")",
        "0 | strftime(\"%Y\", \"%A\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Y\", \"%A\")",
        "0 | gmtime | strftime(\"%c\")",
        "0 | localtime | strflocaltime(\"%c\")",
        "0 | strftime(\"%c\") | strptime(\"%c\")",
        "0 | strflocaltime(\"%c\") | strptime(\"%c\")",
        "\"Thursday\" | strptime(\"%A\")",
        "\"January\" | strptime(\"%B\")",
        "\"Thu\" | strptime(\"%A\")",
        "\"Jan\" | strptime(\"%B\")",
        "\"12 AM\" | strptime(\"%I %p\")",
        "\"12 PM\" | strptime(\"%I %p\")",
        "\"01\" | strptime(\"%I\")",
        "\"4\" | strptime(\"%u\")",
        "\"4\" | strptime(\"%w\")",
        "\"0\" | strptime(\"%s\")",
        "\"1600000000\" | strptime(\"%s\")",
        "\"01/02/70\" | strptime(\"%D\")",
        "\"1/2/70\" | strptime(\"%D\")",
        "\"1970-01-02\" | strptime(\"%F\")",
        "\"1970-01-02 junk\" | strptime(\"%F\")",
        "\"13:05:09\" | strptime(\"%T\")",
        "\"13:05\" | strptime(\"%R\")",
        "\"%\" | strptime(\"%%\")",
        "\"1970%\" | strptime(\"%Y%%\")",
        "\"x%\" | strptime(\"x%%\")",
        "\"Thursday, January 1, 1970\" | strptime(\"%A, %B %d, %Y\")",
        "\"nope\" | strptime(\"%A\")",
        "\"\" | strptime(\"%A\")",
        "\"1970\" | strptime(\"%A\")",
        "\"x\" | strptime(\"%%\")",
        "\"1970\" | strptime(\"%Y\", \"%A\")",
        "true | strftime(\"%A\")",
        "null | strftime(\"%s\")",
        "0 | strptime(\"%F\")",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

/// `%C` `%g` `%G` `%h` `%k` `%l` `%n` `%r` `%t` `%U` `%V` `%W` `%x` `%X`
/// and `%P` are the host C library's letters. Each program is run against
/// the jq on this machine, including a parse that fails.
#[test]
fn host_date_letters_match_jq() {
    require_jq();
    let letters = [
        "C", "g", "G", "h", "k", "l", "n", "r", "t", "U", "V", "W", "x", "X", "P",
    ];
    let stamps = ["0", "46800", "1593561600", "1600000000", "1577836800"];
    let samples = [
        "",
        "0",
        "1",
        "20",
        "2020",
        "Sep",
        "AM",
        "pm",
        " 3",
        "nope",
        "P",
        "am",
        "00",
        "12:00:00 AM",
        "01/01/70",
        "\n",
        "\t",
    ];
    let mut programs = Vec::new();
    for letter in letters {
        let fmt = format!("%{letter}");
        for stamp in stamps {
            programs.push(format!(r#"{stamp} | strftime("{fmt}")"#));
            programs.push(format!(r#"{stamp} | strflocaltime("{fmt}")"#));
            programs.push(format!(
                r#"{stamp} | strftime("{fmt}") | strptime("{fmt}")"#
            ));
            programs.push(format!(
                r#"{stamp} | strflocaltime("{fmt}") | strptime("{fmt}")"#
            ));
        }
        programs.push(format!(r#"[2020,8,13,15,4,0,0,256] | strftime("{fmt}")"#));
        programs.push(format!(
            r#"[2020,8,13,15,4,0,0,256] | strflocaltime("{fmt}")"#
        ));
        // A Rust format string turns `%%` into one `%`, so four of them
        // are the jq program's `%%`.
        programs.push(format!(r#"0 | strftime("%%%%{letter}")"#));
        programs.push(format!(
            r#"0 | strftime("{fmt}%%%%") | strptime("{fmt}%%%%")"#
        ));
        for sample in samples {
            programs.push(format!(
                "{} | strptime(\"{fmt}\")",
                jq_string_literal(sample)
            ));
        }
    }
    let all = "%C %g %G %h %k %l %n %r %t %U %V %W %x %X %P";
    for stamp in ["0", "1600000000"] {
        programs.push(format!(r#"{stamp} | strftime("{all}")"#));
        programs.push(format!(r#"{stamp} | strflocaltime("{all}")"#));
        programs.push(format!(
            r#"{stamp} | strftime("{all}") | strptime("{all}")"#
        ));
        programs.push(format!(
            r#"{stamp} | strflocaltime("{all}") | strptime("{all}")"#
        ));
    }
    programs.push(format!(r#"[2020,8,13,15,4,0,0,256] | strftime("{all}")"#));
    for program in &programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

fn jq_string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' | '"' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Locale-dependent letters follow `LC_ALL`, then `LC_TIME`, then `LANG`.
/// Every name is required. glibc accepts both codeset spellings. Apple
/// ships `UTF-8` only, and `/usr/share/locale` is on a read-only volume,
/// so the `utf8` names are links reached through `PATH_LOCALE`.
#[test]
fn date_locale_matches_jq() {
    require_jq();
    let aliases = utf8_locale_aliases();
    let mut missing = Vec::new();
    for name in DATE_LOCALES {
        match locale_problem(name, aliases.as_deref()) {
            None => {}
            Some(problem) => missing.push(problem),
        }
    }
    assert!(
        missing.is_empty(),
        "missing locales: {}",
        missing.join(", ")
    );
    for name in DATE_LOCALES {
        for program in DATE_LOCALE_PROGRAMS {
            compare_date_locale(program, &[("LC_ALL", name)], aliases.as_deref());
        }
    }
    let en = "en_US.UTF-8";
    let de = "de_DE.UTF-8";
    for program in DATE_LOCALE_CHOICE {
        compare_date_locale(program, &[("LANG", en)], aliases.as_deref());
        compare_date_locale(program, &[("LC_TIME", en)], aliases.as_deref());
        compare_date_locale(
            program,
            &[("LC_ALL", "C"), ("LC_TIME", en), ("LANG", en)],
            aliases.as_deref(),
        );
        compare_date_locale(program, &[("LC_ALL", ""), ("LANG", en)], aliases.as_deref());
        compare_date_locale(
            program,
            &[("LC_TIME", de), ("LANG", en)],
            aliases.as_deref(),
        );
        compare_date_locale(
            program,
            &[("LC_ALL", en), ("LC_TIME", de)],
            aliases.as_deref(),
        );
    }
    for program in DATE_LOCALE_CHOICE {
        compare_date_locale(program, &[("LC_ALL", "zz_ZZ.UTF-8")], aliases.as_deref());
        compare_date_locale(program, &[], aliases.as_deref());
    }
}

/// Names compared above. Both spellings are required. glibc accepts
/// `utf8` and `UTF-8`. Apple ships `UTF-8` only; the other spelling is
/// added by `utf8_locale_aliases`.
const DATE_LOCALES: &[&str] = &[
    "C",
    "POSIX",
    "C.UTF-8",
    "C.utf8",
    "en_US.UTF-8",
    "en_US.utf8",
    "en_GB.UTF-8",
    "en_GB.utf8",
    "de_DE.UTF-8",
    "de_DE.utf8",
    "fr_FR.UTF-8",
    "fr_FR.utf8",
    "ja_JP.UTF-8",
    "ja_JP.utf8",
];

const DATE_LOCALE_PROGRAMS: &[&str] = &[
    r#"0 | strftime("%x")"#,
    r#"0 | strftime("%X")"#,
    r#"0 | strftime("%c")"#,
    r#"0 | strftime("%r")"#,
    r#"0 | strftime("%p")"#,
    r#"0 | strftime("%a %A %b %B %h %P")"#,
    r#"0 | strflocaltime("%x")"#,
    r#"0 | strflocaltime("%c")"#,
    r#"46800 | strftime("%x %X %r %p %P")"#,
    r#"46800 | strflocaltime("%x %X %r")"#,
    r#"1600000000 | strftime("%x %c %B")"#,
    r#"1600000000 | strflocaltime("%x %c")"#,
    r#"[2020,8,13,15,4,0,0,256] | strftime("%x %B")"#,
    r#"0 | strftime("%x") | strptime("%x")"#,
    r#"0 | strftime("%c") | strptime("%c")"#,
    r#"0 | strftime("%B") | strptime("%B")"#,
    r#"0 | strflocaltime("%x") | strptime("%x")"#,
    r#""01/01/70" | strptime("%x")"#,
    r#""01/01/1970" | strptime("%x")"#,
    r#""January" | strptime("%B")"#,
    r#""Januar" | strptime("%B")"#,
    r#""janvier" | strptime("%B")"#,
];

const DATE_LOCALE_CHOICE: &[&str] = &[
    r#"0 | strftime("%x")"#,
    r#"0 | strftime("%B")"#,
    r#"0 | strftime("%c")"#,
    r#"0 | strflocaltime("%x")"#,
    r#"0 | strftime("%B") | strptime("%B")"#,
    r#""January" | strptime("%B")"#,
    r#""Januar" | strptime("%B")"#,
    r#"0 | strftime("%x") | strptime("%x")"#,
];

const LOCALE_ENV: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_TIME",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
];

/// Links whose names are the `utf8` spelling of each shipped `UTF-8` locale.
/// `None` where the C library already accepts that spelling. `PATH_LOCALE`
/// replaces the locale search directory, so it is set only for a `utf8` name.
#[cfg(target_os = "macos")]
fn utf8_locale_aliases() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("date-locales");
    fs::create_dir_all(&root)
        .unwrap_or_else(|err| panic!("could not create {}: {err}", root.display()));
    for name in DATE_LOCALES {
        let Some(spec) = name.strip_suffix(".utf8") else {
            continue;
        };
        let src = format!("/usr/share/locale/{spec}.UTF-8");
        assert!(Path::new(&src).is_dir(), "missing locale directory {src}");
        let dst = root.join(name);
        if dst.symlink_metadata().is_ok() {
            continue;
        }
        std::os::unix::fs::symlink(&src, &dst)
            .unwrap_or_else(|err| panic!("could not link {} to {src}: {err}", dst.display()));
    }
    Some(root)
}

#[cfg(not(target_os = "macos"))]
fn utf8_locale_aliases() -> Option<PathBuf> {
    None
}

fn apply_path_locale(cmd: &mut Command, aliases: Option<&Path>, names: &[&str]) {
    cmd.env_remove("PATH_LOCALE");
    let Some(dir) = aliases else {
        return;
    };
    if names.iter().any(|name| name.ends_with(".utf8")) {
        cmd.env("PATH_LOCALE", dir);
    }
}

/// `utf8` and `UTF-8` are the same codeset. glibc prints one spelling and
/// Apple can print the directory a link points at.
fn same_locale(requested: &str, reported: &str) -> bool {
    reported == requested
        || reported == requested.replacen("utf8", "UTF-8", 1)
        || reported == requested.replacen("UTF-8", "utf8", 1)
}

/// `None` when `name` is installed. glibc prints the requested name and a
/// "Cannot set" warning when it is not. Apple reports the C locale instead.
fn locale_problem(name: &str, aliases: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new("/usr/bin/locale");
    cmd.env_clear().env("LC_ALL", name);
    apply_path_locale(&mut cmd, aliases, &[name]);
    let output = cmd
        .output()
        .unwrap_or_else(|err| panic!("could not run locale: {err}"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Cannot set") {
        return Some(format!("{name} (not installed)"));
    }
    assert!(
        output.status.success(),
        "locale failed for {name}: {stderr}"
    );
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(value) = line.strip_prefix("LC_ALL=") else {
            continue;
        };
        let reported = value.trim().trim_matches('"');
        if same_locale(name, reported) {
            return None;
        }
        return Some(format!("{name} (reports {reported})"));
    }
    Some(format!("{name} (no LC_ALL line)"))
}

fn compare_date_locale(program: &str, vars: &[(&str, &str)], aliases: Option<&Path>) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let label = if vars.is_empty() {
        "no locale variables".to_string()
    } else {
        vars.iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut ours = Command::new(bin);
    let mut theirs = Command::new("jq");
    ours.args(args);
    theirs.args(args);
    for key in LOCALE_ENV {
        ours.env_remove(key);
        theirs.env_remove(key);
    }
    for (key, value) in vars {
        ours.env(key, value);
        theirs.env(key, value);
    }
    let names = vars.iter().map(|(_, value)| *value).collect::<Vec<_>>();
    apply_path_locale(&mut ours, aliases, &names);
    apply_path_locale(&mut theirs, aliases, &names);
    let got = ours
        .output()
        .unwrap_or_else(|err| panic!("could not start tumulus: {err}"));
    let expected = theirs
        .output()
        .unwrap_or_else(|err| panic!("could not start jq: {err}"));
    assert_eq!(
        got.stdout,
        expected.stdout,
        "{program}: stdout differs under {label}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        got.stderr, expected.stderr,
        "{program}: stderr differs under {label}"
    );
    assert_eq!(
        got.status.code(),
        expected.status.code(),
        "{program}: exit status differs under {label}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
}

/// jq aborts when `strftime` is given a number and a format that is not a
/// string. The message here is the one jq prints for an array. Letters
/// other than the ones these filters accept are not compared with jq.
#[test]
fn strftime_letters_are_limited() {
    let cases = [
        (r#"0 | strftime("%")"#, "strftime does not support %"),
        (r#"0 | strftime("%Y%")"#, "strftime does not support %"),
        (r#"0 | strftime("%EY")"#, "strftime does not support %E"),
        (r#"0 | strftime("%-Y")"#, "strftime does not support %-"),
        (r#"0 | strftime("%10Y")"#, "strftime does not support %1"),
        (r#"0 | strftime("%Q%Y")"#, "strftime does not support %Q"),
        (r#"0 | strftime(1)"#, "strftime/1 requires a string format"),
        (
            r#"0 | strftime(null)"#,
            "strftime/1 requires a string format",
        ),
        (
            r#"0 | strftime(true)"#,
            "strftime/1 requires a string format",
        ),
        (
            r#"0 | strftime(["%Y"])"#,
            "strftime/1 requires a string format",
        ),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// jq aborts when `strflocaltime` converts a number and the format is not a
/// string, and when that conversion fails. The message is the one jq prints
/// for an array, or the converting error. Letters other than the ones these
/// filters accept are not compared with jq.
#[test]
fn strflocaltime_letters_are_limited() {
    let cases = [
        (
            r#"0 | strflocaltime("%")"#,
            "strflocaltime does not support %",
        ),
        (
            r#"0 | strflocaltime("%Y%")"#,
            "strflocaltime does not support %",
        ),
        (
            r#"0 | strflocaltime("%EY")"#,
            "strflocaltime does not support %E",
        ),
        (
            r#"0 | strflocaltime("%-Y")"#,
            "strflocaltime does not support %-",
        ),
        (
            r#"0 | strflocaltime("%10Y")"#,
            "strflocaltime does not support %1",
        ),
        (
            r#"0 | strflocaltime("%Q%Y")"#,
            "strflocaltime does not support %Q",
        ),
        (
            r#"0 | strflocaltime(1)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(null)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(true)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(["%Y"])"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"infinite | strflocaltime("%Y")"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"infinite | strflocaltime(1)"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"-infinite | strflocaltime("%Y")"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"-infinite | strflocaltime(1)"#,
            "error converting number of seconds since epoch to datetime",
        ),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// `strptime` rejects the same letters `strftime` rejects. jq would hand
/// those letters to the C library, so the message is not compared with jq.
#[test]
fn strptime_letters_are_limited() {
    let cases = [
        (r#""1970" | strptime("%")"#, "strptime does not support %"),
        (r#""1970" | strptime("%Y%")"#, "strptime does not support %"),
        (
            r#""1970" | strptime("%Ey")"#,
            "strptime does not support %E",
        ),
        (
            r#""1970" | strptime("%-y")"#,
            "strptime does not support %-",
        ),
        (
            r#""1970" | strptime("%3j")"#,
            "strptime does not support %3",
        ),
        (r#""1970" | strptime("%Q")"#, "strptime does not support %Q"),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// The two processes read the clock a moment apart. The number itself is
/// not compared with jq.
#[test]
fn now_is_close_to_jq() {
    require_jq();
    let args = ["-n", "-c", "--", "now"];
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(env!("CARGO_BIN_EXE_tumulus"), &owned, b"");
    let expected = run("jq", &owned, b"");
    let ours = one_stamp(&got);
    let theirs = one_stamp(&expected);
    let gap = (ours - theirs).abs();
    assert!(
        gap < 5.0,
        "now differs by {gap}: tumulus {ours}, jq {theirs}"
    );
}

fn one_stamp(output: &std::process::Output) -> f64 {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = std::str::from_utf8(&output.stdout).expect("utf-8");
    let stamp: f64 = text
        .trim()
        .parse()
        .unwrap_or_else(|err| panic!("now printed {text:?}: {err}"));
    assert!(stamp.is_finite() && stamp > 1_600_000_000.0, "{stamp}");
    stamp
}

fn compare_tz(name: &str, program: &str, tz: &str) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let got = Command::new(bin)
        .args(args)
        .env("TZ", tz)
        .output()
        .unwrap_or_else(|err| panic!("could not start tumulus: {err}"));
    let expected = Command::new("jq")
        .args(args)
        .env("TZ", tz)
        .output()
        .unwrap_or_else(|err| panic!("could not start jq: {err}"));
    assert_eq!(
        got.stdout,
        expected.stdout,
        "{name}: stdout differs under TZ={tz}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        got.stderr, expected.stderr,
        "{name}: stderr differs under TZ={tz}"
    );
    assert_eq!(
        got.status.code(),
        expected.status.code(),
        "{name}: exit status differs under TZ={tz}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
}
