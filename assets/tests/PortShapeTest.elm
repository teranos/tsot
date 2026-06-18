module PortShapeTest exposing (suite)

{-| Port-payload shape allowlist. Counterpart to Rust's
`TRACE_STRING_ALLOWLIST` (`src/trace.rs`), which catches new
stringly-typed fields in the engine's `TraceEvent` enum. This test
does the analogous job on the Elm side: every inbound port has a
canonical sample payload checked here, decoded with the production
decoder. If the engine changes a port wire shape WITHOUT updating
the sample below, this test fails — making the drift reviewable in
the diff.

Per ERROR.md "Self-enforcement holes": port-payload shapes can drift
silently without a test catching the case that Main.elm's 7 typed
decode-error sites have to defend against today.

The sample for each port is a JSON literal embedded here. Updating
the engine wire shape requires updating the sample, which sets a
floor on schema-drift visibility.

If you add a new inbound port, add:

  1. A `describe "<port-name>"` block below with a canonical sample.
  2. A `test "decodes canonical sample"` that runs the production
     decoder on the sample and asserts `Ok _`.
  3. (Optional) negative cases — known-malformed payloads that the
     decoder must REJECT, asserting decode returns `Err _`.

This file is the place to discover what shapes Elm accepts without
reading every decoder source. Treat it as the schema-as-tests.

-}

import Error
import Expect
import Json.Decode as D
import LogPanel
import Test exposing (Test, describe, test)



-- ====================================================================
-- errorIn — the typed Error envelope from JS (tsotPushError) + Rust
-- (envelope.errors[]).  Canonical wire shape is owned by the
-- `sacred-error` crate; see `crates/sacred-error/src/lib.rs`.
-- ====================================================================


errorInSample : String
errorInSample =
    """
    {
      "id": "err-rust-42",
      "severity": "error",
      "context": {
        "surface": "deckbuilder",
        "region": "preset-dropdown",
        "anchor": { "x": 120, "y": 80 }
      },
      "title": "deck preset rejected",
      "why": "preset[2].cards is empty",
      "trace": ["build_preset_decks", "validate_preset"],
      "raw": "{\\\"id\\\":\\\"x\\\",\\\"cards\\\":[]}",
      "at": "1718640000us"
    }
    """


errorInMinimalSample : String
errorInMinimalSample =
    """
    {
      "id": "err-rust-1",
      "severity": "warn",
      "context": { "surface": "engine" },
      "title": "missing card in zone",
      "why": "iid-9999 not in Hand",
      "at": ""
    }
    """


errorInRejectsUnknownSeverity : String
errorInRejectsUnknownSeverity =
    """
    {
      "id": "err-x-1",
      "severity": "fatal",
      "context": { "surface": "test" },
      "title": "x",
      "why": "x",
      "at": ""
    }
    """



-- ====================================================================
-- logErrorIn — LogPanel.ErrorEvent, the legacy log-panel error path.
-- Will collapse into Error.view per Slice 1 deferred bullet.
-- ====================================================================


logErrorInSample : String
logErrorInSample =
    """
    {
      "source": "rust-panic",
      "message": "panicked at src/game/play.rs:42",
      "location": "src/game/play.rs:42:13",
      "ffi_call": "tsot_apply_action",
      "at_us": 12345.6,
      "breadcrumb": ["Step", "Cursor"],
      "js_stack": null,
      "raw_stderr": "panicked at..."
    }
    """


logErrorInMinimalSample : String
logErrorInMinimalSample =
    """
    {}
    """



suite : Test
suite =
    describe "Port payload shapes"
        [ describe "errorIn (typed Error envelope)"
            [ test "full sample decodes" <|
                \_ ->
                    case D.decodeString Error.decode errorInSample of
                        Ok _ ->
                            Expect.pass

                        Err e ->
                            Expect.fail (D.errorToString e)
            , test "minimal sample (optional fields omitted) decodes" <|
                \_ ->
                    case D.decodeString Error.decode errorInMinimalSample of
                        Ok _ ->
                            Expect.pass

                        Err e ->
                            Expect.fail (D.errorToString e)
            , test "unknown severity FAILS decode (axiom: no silent default)" <|
                \_ ->
                    case D.decodeString Error.decode errorInRejectsUnknownSeverity of
                        Ok _ ->
                            Expect.fail
                                "axiom violation: unknown severity must FAIL decode, not silently default"

                        Err _ ->
                            Expect.pass
            ]
        , describe "logErrorIn (LogPanel.ErrorEvent — legacy path)"
            [ test "full sample decodes" <|
                \_ ->
                    case D.decodeString LogPanel.decodeError logErrorInSample of
                        Ok _ ->
                            Expect.pass

                        Err e ->
                            Expect.fail (D.errorToString e)
            , test "all-optional-fields-omitted sample decodes (Maybe.withDefault legitimate)" <|
                \_ ->
                    case D.decodeString LogPanel.decodeError logErrorInMinimalSample of
                        Ok _ ->
                            Expect.pass

                        Err e ->
                            Expect.fail (D.errorToString e)
            ]
        , describe "errorRestoreIn (Slice 6 — localStorage payload on boot)"
            [ test "empty array decodes" <|
                \_ ->
                    case D.decodeString (D.list Error.decode) "[]" of
                        Ok [] ->
                            Expect.pass

                        Ok _ ->
                            Expect.fail "[] should decode to an empty list"

                        Err e ->
                            Expect.fail (D.errorToString e)
            , test "array of two Errors round-trips, both with original ids" <|
                \_ ->
                    let
                        sample : String
                        sample =
                            "[" ++ errorInMinimalSample ++ "," ++ errorInSample ++ "]"
                    in
                    case D.decodeString (D.list Error.decode) sample of
                        Ok errs ->
                            -- Bijectivity invariant: ids survive
                            -- round-trip. Slice 6's whole point —
                            -- the persisted DOM key is the original
                            -- id, not a freshly-minted counter.
                            case errs of
                                [ a, b ] ->
                                    Expect.all
                                        [ \_ -> Expect.equal a.id "err-rust-1"
                                        , \_ -> Expect.equal b.id "err-rust-42"
                                        ]
                                        ()

                                _ ->
                                    Expect.fail
                                        "expected exactly two Errors"

                        Err e ->
                            Expect.fail (D.errorToString e)
            ]
        ]
