module ErrorTest exposing (suite)

{-| Tests for the `Error` primitive. Pins the wire shape, the
severity vocabulary, the keyed-identity contract, and the decoder
error path so the failure surface is itself uncolored by its own
silent drops.

The tests intentionally cover the CASES that turn into developer-
visible failures when they regress:

  - decode round-trip across all four severities
  - optional `region`, `trace`, `raw` decode cleanly when absent
  - decoder errors with a useful message when severity is unknown
    (so we don't silently default it — defaulting severity would
    violate the axiom: an unknown failure mode is itself an error)
  - `key` returns `id` so `Html.Keyed` containers preserve identity

-}

import Error exposing (Error, Severity(..))
import Expect
import Json.Decode as D
import Json.Encode as E
import Test exposing (Test, describe, test)


suite : Test
suite =
    describe "Error primitive"
        [ describe "decode"
            [ test "decodes the full shape with every field set" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "id", E.string "err-0001" )
                                , ( "severity", E.string "error" )
                                , ( "context"
                                  , E.object
                                        [ ( "surface", E.string "deckbuilder" )
                                        , ( "region", E.string "preset-dropdown" )
                                        ]
                                  )
                                , ( "title", E.string "bootDataIn decode failed" )
                                , ( "why", E.string "Expected non-empty cards array at presets[2].cards" )
                                , ( "trace"
                                  , E.list E.string
                                        [ "Cursor PatternBPick -> PatternBResolving"
                                        , "Oracle choose_card asker=A -> Pending"
                                        ]
                                  )
                                , ( "raw", E.string "{\"id\":\"yield-test\",\"cards\":[]}" )
                                , ( "at", E.string "2026-06-10T19:00:00Z" )
                                ]
                    in
                    case D.decodeValue Error.decode json of
                        Ok e ->
                            Expect.all
                                [ \v -> Expect.equal v.id "err-0001"
                                , \v -> Expect.equal v.severity LevelError
                                , \v -> Expect.equal v.context.surface "deckbuilder"
                                , \v -> Expect.equal v.context.region (Just "preset-dropdown")
                                , \v -> Expect.equal v.context.anchor Nothing
                                , \v -> Expect.equal v.title "bootDataIn decode failed"
                                , \v ->
                                    Expect.equal v.why
                                        "Expected non-empty cards array at presets[2].cards"
                                , \v -> Expect.equal (List.length v.trace) 2
                                , \v -> Expect.equal v.raw (Just "{\"id\":\"yield-test\",\"cards\":[]}")
                                , \v -> Expect.equal v.at "2026-06-10T19:00:00Z"
                                ]
                                e

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "decodes the click-cursor anchor when present" <|
                \_ ->
                    -- ERROR.md primary case: failing click action carries
                    -- the cursor coordinates so the overlay opens AT the
                    -- click point. Capture via Error.clickAnchorDecoder
                    -- on the originating event, then serialize as
                    -- { anchor: { x, y } } in the Error wire shape.
                    let
                        json =
                            E.object
                                [ ( "id", E.string "err-click-001" )
                                , ( "severity", E.string "error" )
                                , ( "context"
                                  , E.object
                                        [ ( "surface", E.string "your-board" )
                                        , ( "region", E.string "card:A:0007" )
                                        , ( "anchor"
                                          , E.object
                                                [ ( "x", E.float 412.5 )
                                                , ( "y", E.float 271.0 )
                                                ]
                                          )
                                        ]
                                  )
                                , ( "title", E.string "PlayCard failed: SymbolCastCapReached" )
                                , ( "why", E.string "Already cast 1 Symbol this turn (P.35)" )
                                , ( "at", E.string "2026-06-10T19:02:00Z" )
                                ]
                    in
                    case D.decodeValue Error.decode json of
                        Ok e ->
                            case e.context.anchor of
                                Just a ->
                                    Expect.all
                                        [ \v -> Expect.within (Expect.Absolute 0.001) v.x 412.5
                                        , \v -> Expect.within (Expect.Absolute 0.001) v.y 271.0
                                        ]
                                        a

                                Nothing ->
                                    Expect.fail "expected anchor to decode"

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "clickAnchorDecoder pulls clientX/clientY off a DOM-shaped event" <|
                \_ ->
                    -- The Elm view layer captures MouseEvent via
                    -- `Html.Events.on "click" clickAnchorDecoder`; the
                    -- decoder must match the shape the browser delivers.
                    let
                        mouseEvent =
                            E.object
                                [ ( "clientX", E.float 200.0 )
                                , ( "clientY", E.float 150.0 )
                                ]
                    in
                    case D.decodeValue Error.clickAnchorDecoder mouseEvent of
                        Ok a ->
                            Expect.all
                                [ \v -> Expect.within (Expect.Absolute 0.001) v.x 200.0
                                , \v -> Expect.within (Expect.Absolute 0.001) v.y 150.0
                                ]
                                a

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "treats trace + region + raw as optional (absent decodes cleanly)" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "id", E.string "err-0002" )
                                , ( "severity", E.string "warn" )
                                , ( "context", E.object [ ( "surface", E.string "log" ) ] )
                                , ( "title", E.string "noisy event" )
                                , ( "why", E.string "filler" )
                                , ( "at", E.string "2026-06-10T19:01:00Z" )
                                ]
                    in
                    case D.decodeValue Error.decode json of
                        Ok e ->
                            Expect.all
                                [ \v -> Expect.equal v.severity Warn
                                , \v -> Expect.equal v.context.region Nothing
                                , \v -> Expect.equal v.trace []
                                , \v -> Expect.equal v.raw Nothing
                                ]
                                e

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "decodes each known severity case-insensitively" <|
                \_ ->
                    let
                        decodeWithSeverity s =
                            let
                                json =
                                    E.object
                                        [ ( "id", E.string "x" )
                                        , ( "severity", E.string s )
                                        , ( "context", E.object [ ( "surface", E.string "x" ) ] )
                                        , ( "title", E.string "x" )
                                        , ( "why", E.string "x" )
                                        , ( "at", E.string "x" )
                                        ]
                            in
                            D.decodeValue Error.decode json |> Result.map .severity
                    in
                    Expect.all
                        [ \_ -> Expect.equal (decodeWithSeverity "info") (Ok Info)
                        , \_ -> Expect.equal (decodeWithSeverity "INFO") (Ok Info)
                        , \_ -> Expect.equal (decodeWithSeverity "warn") (Ok Warn)
                        , \_ -> Expect.equal (decodeWithSeverity "warning") (Ok Warn)
                        , \_ -> Expect.equal (decodeWithSeverity "Warning") (Ok Warn)
                        , \_ -> Expect.equal (decodeWithSeverity "error") (Ok LevelError)
                        , \_ -> Expect.equal (decodeWithSeverity "Error") (Ok LevelError)
                        , \_ -> Expect.equal (decodeWithSeverity "panic") (Ok Panic)
                        , \_ -> Expect.equal (decodeWithSeverity "PANIC") (Ok Panic)
                        ]
                        ()
            , test "decode FAILS on unknown severity rather than defaulting silently" <|
                \_ ->
                    -- ERROR.md axiom: an unknown failure mode is itself
                    -- a failure. Silently defaulting an unknown
                    -- severity to "error" would hide upstream wire
                    -- drift.
                    let
                        json =
                            E.object
                                [ ( "id", E.string "x" )
                                , ( "severity", E.string "definitely-not-a-severity" )
                                , ( "context", E.object [ ( "surface", E.string "x" ) ] )
                                , ( "title", E.string "x" )
                                , ( "why", E.string "x" )
                                , ( "at", E.string "x" )
                                ]
                    in
                    case D.decodeValue Error.decode json of
                        Ok _ ->
                            Expect.fail
                                "decoder should fail on unknown severity, not silently default"

                        Err _ ->
                            Expect.pass
            ]
        , describe "key"
            [ test "returns the id field so Html.Keyed containers preserve identity" <|
                \_ ->
                    let
                        e =
                            sampleError "err-key-test"
                    in
                    Expect.equal (Error.key e) "err-key-test"
            ]
        ]


sampleError : String -> Error
sampleError id_ =
    { id = id_
    , severity = LevelError
    , context = { surface = "test", region = Nothing, anchor = Nothing }
    , title = "t"
    , why = "w"
    , trace = []
    , raw = Nothing
    , at = "t"
    }
