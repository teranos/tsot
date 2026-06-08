port module Main exposing (main)

{-| H7-Elm Stage 4 — the decision-report panel is rendered by Elm.

The dev-tool records, per human prompt, a snapshot of {what UCT
preferred, what the human chose, whether they agreed}; on game end,
a summary record carries the winner. The "Decision report" button
reads every record back from `IndexedDB.decision_log`, aggregates
agreement rates and per-card UCT-vs-human win counts, and renders
the table inline. Stage 4 moves the panel + aggregator from inline
JS into Elm and introduces:

  - the first **outbound** port (Elm → JS): `decisionFetchOut`.
    The dev tool's first proof that Elm can issue commands, not
    just receive events. Stages 1-3 only validated the inbound
    direction.
  - the first **IndexedDB** access from Elm. js-bridge.js owns the
    DB-open helper + the read-path queries; play.html still owns the
    write path (`recordDecision` calls `dbAppendDecision` inline).
    During the transition both sides open the same `tsot` v2 DB with
    the same `saves` + `decision_log` stores — no schema drift can
    occur because both sites agree on the upgrade.

Three buttons still live in `play.html`'s `<div id="save-controls">`
(they share the bar with Save/Load/Test-panic which haven't been
ported yet). Their `onclick` handlers route through three JS shims
exposed by `assets/js-bridge.js`:

    window.tsotDecisionReport()   → app.ports.decisionReportClickedIn
    window.tsotDecisionExport()   → IDB read + Blob download (no Elm)
    window.tsotDecisionClear()    → IDB clear + save-status (no Elm)

Export + Clear are intentionally bypassing Elm — they don't change
visible Elm state (the panel only re-fetches on Report click), so
routing them through Elm would add ports with no benefit. Save-status
text writes to the existing `<span id="save-status">` directly via
DOM — that span will move to Elm in a later Stage when the rest of
the save controls port.

Panel state machine (`DecisionPanel`):

    DecisionHidden       — initial state, also the toggled-off state.
    DecisionLoading      — outbound port fired, waiting for records.
    DecisionShown agg    — records arrived + aggregated, panel rendered.
    DecisionError msg    — decoder failed; surface the error in-panel.

Toggle behavior: clicking Decision report cycles `Hidden → Loading
→ Shown`; clicking again from `Shown` (or `Error`) returns to
`Hidden`. The in-Elm Close button on the panel itself also returns
to `Hidden`.

-}

import Browser
import Browser.Dom
import Dict exposing (Dict)
import Html exposing (Html, button, div, h2, pre, span, table, td, text, th, tr)
import Html.Attributes exposing (class, id, style)
import Html.Events exposing (onClick)
import Json.Decode as D
import Json.Encode as E
import Task



-- PORTS


port buildInfoIn : (D.Value -> msg) -> Sub msg


port logTextIn : (String -> msg) -> Sub msg


port logErrorIn : (D.Value -> msg) -> Sub msg


port decisionLogIn : (D.Value -> msg) -> Sub msg


port savedListIn : (D.Value -> msg) -> Sub msg


port saveStatusIn : (String -> msg) -> Sub msg


port gamePhaseIn : (String -> msg) -> Sub msg


{-| One outbound port for every worker-bound action. Carries an
`{cmd, payload}` envelope. cmd is the operation name
(`"save_game"` / `"download"` / `"load_from_file"` / `"test_panic"` /
`"start_game"` / `"start_spectate"`); payload carries args when the
op needs them (else `E.null`). js-bridge dispatches by cmd string;
unknown cmds throw and surface via the fault-surface diagnostic.
-}
port workerCmdOut : { cmd : String, payload : E.Value } -> Cmd msg


{-| Bootstrap data from play.html — the card pool + preset decks the
worker returned during startup (`list_card_pool` + `list_preset_decks`).
Sent once on page load. Carries `{cardPool : [...], presets : [...]}`.
-}
port bootDataIn : (D.Value -> msg) -> Sub msg


{-| One outbound port for every IDB-bound action. Carries an
`{op, payload}` envelope. Per-feature ports collapsed here:
`decision_get_all` / `decision_export` / `decision_clear` /
`saved_get_all` / `saved_item_action` (the last carries
`{action, id}` in its payload).
-}
port idbReqOut : { op : String, payload : E.Value } -> Cmd msg



-- MODEL


type alias BuildInfo =
    { profile : String
    , builtAt : String
    , commit : String
    }


type BuildState
    = AwaitingPort
    | NoBuildInfo
    | HasBuildInfo BuildInfo


type alias ErrorEvent =
    { source : String
    , message : String
    , location : Maybe String
    , ffiCall : Maybe String
    , atUs : Maybe Float
    , breadcrumb : List String
    , jsStack : Maybe String
    , rawStderr : Maybe String
    }


type LogEntry
    = TextLine String
    | ErrorEntry ErrorEvent


type DecisionRecord
    = PromptRec PromptDetails
    | SummaryRec SummaryDetails
    | UnknownRec


type alias PromptDetails =
    { gameId : String
    , hasUct : Bool
    , uctChosen : Maybe String
    , agreement : Maybe Bool
    , humanCard : Maybe String
    }


type alias SummaryDetails =
    { gameId : String
    , winner : Maybe String
    }


type alias PerCardRow =
    { card : String
    , uctRecommended : Int
    , humanPicked : Int
    , wins : Int
    , decidedGames : Int
    }


type alias DecisionAggregation =
    { totalRecords : Int
    , nGames : Int
    , nGamesWithSummary : Int
    , nPrompts : Int
    , nUctPrompts : Int
    , nAgree : Int
    , nDisagree : Int
    , agreeWins : Int
    , agreeLosses : Int
    , disagreeWins : Int
    , disagreeLosses : Int
    , perCard : List PerCardRow
    }


type DecisionPanel
    = DecisionHidden
    | DecisionLoading
    | DecisionShown DecisionAggregation
    | DecisionError String


type alias SaveItem =
    { id : Int
    , name : String
    , savedAt : String
    }


type SavedListState
    = SavedHidden
    | SavedLoading
    | SavedShown (List SaveItem)
    | SavedError String


type GamePhase
    = Deckbuilding
    | Playing
    | Spectating
    | UnknownPhase


type alias CardPoolEntry =
    { id : String
    , name : String
    , kind : String
    , costText : String
    , colors : List String
    , symbols : List String
    , subtypes : List String
    , power : Maybe Float
    , toughness : Maybe Float
    , timing : Maybe String
    , abilities : List String
    }


type alias PresetDeck =
    { id : String
    , name : String
    , cards : List String
    }


type alias Model =
    { build : BuildState
    , log : List LogEntry
    , decisionPanel : DecisionPanel
    , savedList : SavedListState
    , saveStatus : String
    , gamePhase : GamePhase
    , cardPool : List CardPoolEntry
    , presets : List PresetDeck
    , deck : List String
    , oppAi : String
    , specAiA : String
    , specAiB : String
    , poolFilterColor : String
    , poolFilterKind : String
    }


logContainerId : String
logContainerId =
    "elm-log"



-- MSG


type Msg
    = BuildInfoReceived D.Value
    | LogTextReceived String
    | LogErrorReceived D.Value
    | DecisionReportClicked
    | DecisionExportClicked
    | DecisionClearClicked
    | DecisionLogReceived D.Value
    | DecisionPanelClosed
    | SavedListToggleClicked
    | SavedListReceived D.Value
    | SavedItemLoad Int
    | SavedItemDownload Int
    | SavedItemDelete Int
    | SavedListClosed
    | SaveClicked
    | DownloadClicked
    | LoadFromFileClicked
    | TestPanicClicked
    | SaveStatusReceived String
    | GamePhaseReceived String
    | BootDataReceived D.Value
    | PoolCardClicked String
    | DeckRowRemove String
    | DeckClearClicked
    | PresetChosen String
    | OppAiChanged String
    | SpecAiAChanged String
    | SpecAiBChanged String
    | PoolFilterColorChanged String
    | PoolFilterKindChanged String
    | StartGameClicked
    | StartSpectateClicked
    | NoOp



-- INIT


init : () -> ( Model, Cmd Msg )
init _ =
    ( { build = AwaitingPort
      , log = []
      , decisionPanel = DecisionHidden
      , savedList = SavedHidden
      , saveStatus = ""
      , gamePhase = UnknownPhase
      , cardPool = []
      , presets = []
      , deck = []
      , oppAi = "uct"
      , specAiA = "uct"
      , specAiB = "uct"
      , poolFilterColor = ""
      , poolFilterKind = ""
      }
    , Cmd.none
    )


savedItemPayload : String -> Int -> E.Value
savedItemPayload action id =
    E.object [ ( "action", E.string action ), ( "id", E.int id ) ]


parseGamePhase : String -> GamePhase
parseGamePhase s =
    case s of
        "deckbuilding" ->
            Deckbuilding

        "playing" ->
            Playing

        "spectating" ->
            Spectating

        _ ->
            UnknownPhase



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        BuildInfoReceived value ->
            case D.decodeValue decodeBuildInfo value of
                Ok info ->
                    ( { model | build = HasBuildInfo info }, Cmd.none )

                Err _ ->
                    ( { model | build = NoBuildInfo }, Cmd.none )

        LogTextReceived line ->
            ( { model | log = model.log ++ [ TextLine line ] }, scrollLogToBottom )

        LogErrorReceived value ->
            case D.decodeValue decodeErrorEvent value of
                Ok ev ->
                    ( { model | log = model.log ++ [ ErrorEntry ev ] }, scrollLogToBottom )

                Err err ->
                    ( { model
                        | log =
                            model.log
                                ++ [ TextLine ("[log decode failed] " ++ D.errorToString err) ]
                      }
                    , scrollLogToBottom
                    )

        DecisionReportClicked ->
            case model.decisionPanel of
                DecisionShown _ ->
                    ( { model | decisionPanel = DecisionHidden }, Cmd.none )

                DecisionError _ ->
                    ( { model | decisionPanel = DecisionHidden }, Cmd.none )

                _ ->
                    ( { model | decisionPanel = DecisionLoading }
                    , idbReqOut { op = "decision_get_all", payload = E.null }
                    )

        DecisionLogReceived value ->
            case D.decodeValue (D.list decodeDecisionRecord) value of
                Ok records ->
                    ( { model | decisionPanel = DecisionShown (aggregate records) }
                    , Cmd.none
                    )

                Err err ->
                    ( { model | decisionPanel = DecisionError (D.errorToString err) }
                    , Cmd.none
                    )

        DecisionExportClicked ->
            ( model, idbReqOut { op = "decision_export", payload = E.null } )

        DecisionClearClicked ->
            ( model, idbReqOut { op = "decision_clear", payload = E.null } )

        DecisionPanelClosed ->
            ( { model | decisionPanel = DecisionHidden }, Cmd.none )

        SavedListToggleClicked ->
            case model.savedList of
                SavedShown _ ->
                    ( { model | savedList = SavedHidden }, Cmd.none )

                SavedError _ ->
                    ( { model | savedList = SavedHidden }, Cmd.none )

                _ ->
                    ( { model | savedList = SavedLoading }
                    , idbReqOut { op = "saved_get_all", payload = E.null }
                    )

        SavedListReceived value ->
            -- A refresh push from JS (after a Save or Delete) arrives
            -- here too. Only update visibility if the panel is shown;
            -- background refresh shouldn't yank a hidden panel open.
            case model.savedList of
                SavedHidden ->
                    ( model, Cmd.none )

                _ ->
                    case D.decodeValue decodeSavedListEnvelope value of
                        Ok (Ok items) ->
                            ( { model | savedList = SavedShown items }, Cmd.none )

                        Ok (Err err) ->
                            ( { model | savedList = SavedError err }, Cmd.none )

                        Err err ->
                            ( { model | savedList = SavedError (D.errorToString err) }
                            , Cmd.none
                            )

        SavedItemLoad id ->
            -- JS reads the record, calls the inline `loadSaveJson`
            -- (still in play.html) which mutates game state + renders.
            -- Elm doesn't track that side; the panel stays Shown so
            -- the user can pick a different save if loading fails.
            ( model, idbReqOut { op = "saved_item_action", payload = savedItemPayload "load" id } )

        SavedItemDownload id ->
            ( model, idbReqOut { op = "saved_item_action", payload = savedItemPayload "download" id } )

        SavedItemDelete id ->
            -- JS asks confirm(), then deletes, then sends the fresh list
            -- back via savedListIn. Elm transitions to Loading so the
            -- user sees the panel is updating.
            ( { model | savedList = SavedLoading }
            , idbReqOut { op = "saved_item_action", payload = savedItemPayload "delete" id }
            )

        SavedListClosed ->
            ( { model | savedList = SavedHidden }, Cmd.none )

        SaveClicked ->
            ( model, workerCmdOut { cmd = "save_game", payload = E.null } )

        DownloadClicked ->
            ( model, workerCmdOut { cmd = "download", payload = E.null } )

        LoadFromFileClicked ->
            ( model, workerCmdOut { cmd = "load_from_file", payload = E.null } )

        TestPanicClicked ->
            ( model, workerCmdOut { cmd = "test_panic", payload = E.null } )

        SaveStatusReceived msgText ->
            ( { model | saveStatus = msgText }, Cmd.none )

        GamePhaseReceived phaseStr ->
            ( { model | gamePhase = parseGamePhase phaseStr }, Cmd.none )

        BootDataReceived value ->
            case D.decodeValue decodeBootData value of
                Ok boot ->
                    let
                        starterCards =
                            boot.presets
                                |> List.filter (\p -> p.id == "starter")
                                |> List.head
                                |> (\m ->
                                        case m of
                                            Just p ->
                                                p.cards

                                            Nothing ->
                                                case List.head boot.presets of
                                                    Just p ->
                                                        p.cards

                                                    Nothing ->
                                                        []
                                   )

                        deck =
                            if List.isEmpty model.deck then
                                starterCards

                            else
                                model.deck
                    in
                    ( { model
                        | cardPool = boot.cardPool
                        , presets = boot.presets
                        , deck = deck
                      }
                    , Cmd.none
                    )

                Err _ ->
                    ( model, Cmd.none )

        PoolCardClicked cardId ->
            ( { model | deck = model.deck ++ [ cardId ] }, Cmd.none )

        DeckRowRemove cardId ->
            ( { model | deck = removeFirst cardId model.deck }, Cmd.none )

        DeckClearClicked ->
            ( { model | deck = [] }, Cmd.none )

        PresetChosen presetId ->
            case List.filter (\p -> p.id == presetId) model.presets |> List.head of
                Just p ->
                    ( { model | deck = p.cards }, Cmd.none )

                Nothing ->
                    ( model, Cmd.none )

        OppAiChanged ai ->
            ( { model | oppAi = ai }, Cmd.none )

        SpecAiAChanged ai ->
            ( { model | specAiA = ai }, Cmd.none )

        SpecAiBChanged ai ->
            ( { model | specAiB = ai }, Cmd.none )

        PoolFilterColorChanged color ->
            ( { model | poolFilterColor = color }, Cmd.none )

        PoolFilterKindChanged kind ->
            ( { model | poolFilterKind = kind }, Cmd.none )

        StartGameClicked ->
            if List.isEmpty model.deck then
                ( model, Cmd.none )

            else
                ( model
                , workerCmdOut
                    { cmd = "start_game"
                    , payload =
                        E.object
                            [ ( "deckIds", E.list E.string model.deck )
                            , ( "oppAi", E.string model.oppAi )
                            ]
                    }
                )

        StartSpectateClicked ->
            if List.isEmpty model.deck then
                ( model, Cmd.none )

            else
                ( model
                , workerCmdOut
                    { cmd = "start_spectate"
                    , payload =
                        E.object
                            [ ( "deckIds", E.list E.string model.deck )
                            , ( "aiA", E.string model.specAiA )
                            , ( "aiB", E.string model.specAiB )
                            ]
                    }
                )

        NoOp ->
            ( model, Cmd.none )


removeFirst : a -> List a -> List a
removeFirst target list =
    case list of
        [] ->
            []

        x :: xs ->
            if x == target then
                xs

            else
                x :: removeFirst target xs


scrollLogToBottom : Cmd Msg
scrollLogToBottom =
    Browser.Dom.setViewportOf logContainerId 0 1000000
        |> Task.attempt (\_ -> NoOp)



-- DECODERS


decodeBuildInfo : D.Decoder BuildInfo
decodeBuildInfo =
    D.map3 BuildInfo
        (D.field "profile" D.string)
        (D.field "builtAt" D.string)
        (D.field "commit" D.string)


decodeErrorEvent : D.Decoder ErrorEvent
decodeErrorEvent =
    D.map8 ErrorEvent
        (optionalField "source" D.string |> D.map (Maybe.withDefault "error"))
        (optionalField "message" D.string |> D.map (Maybe.withDefault "(no message)"))
        (optionalField "location" D.string)
        (optionalField "ffi_call" D.string)
        (optionalField "at_us" D.float)
        (optionalField "breadcrumb" (D.list D.string) |> D.map (Maybe.withDefault []))
        (optionalField "js_stack" D.string)
        (optionalField "raw_stderr" D.string)


decodeDecisionRecord : D.Decoder DecisionRecord
decodeDecisionRecord =
    D.oneOf
        [ D.field "type" D.string
            |> D.andThen
                (\t ->
                    case t of
                        "prompt" ->
                            D.map PromptRec decodePromptDetails

                        "summary" ->
                            D.map SummaryRec decodeSummaryDetails

                        _ ->
                            D.succeed UnknownRec
                )
        , D.succeed UnknownRec
        ]


decodePromptDetails : D.Decoder PromptDetails
decodePromptDetails =
    D.map5 PromptDetails
        (D.field "gameId" D.string)
        (D.oneOf
            [ D.field "uct" (D.succeed True)
            , D.succeed False
            ]
        )
        (D.maybe (D.at [ "uct", "chosen" ] D.string)
            |> D.map (Maybe.map cardSuffixFromIid)
        )
        (optionalField "agreement" D.bool)
        (optionalField "human_action" decodeHumanCardFromAction)


{-| The original JS only counts `human_action.iid` when
`human_action.kind === 'PlayCard'`. Decoder mirrors that — it fails
quietly on any other shape, leaving `humanCard = Nothing`.
-}
decodeHumanCardFromAction : D.Decoder String
decodeHumanCardFromAction =
    D.field "kind" D.string
        |> D.andThen
            (\k ->
                if k == "PlayCard" then
                    D.field "iid" D.string |> D.map cardSuffixFromIid

                else
                    D.fail "not a PlayCard"
            )


decodeSummaryDetails : D.Decoder SummaryDetails
decodeSummaryDetails =
    D.map2 SummaryDetails
        (D.field "gameId" D.string)
        (optionalField "winner" D.string)


{-| js-bridge sends `{ items: [...] }` on success or `{ error: "..." }`
on failure. The Result-typed envelope keeps the two paths explicit at
the call site in `update`.
-}
decodeSavedListEnvelope : D.Decoder (Result String (List SaveItem))
decodeSavedListEnvelope =
    D.oneOf
        [ D.field "items" (D.list decodeSaveItem) |> D.map Ok
        , D.field "error" D.string |> D.map Err
        ]


decodeSaveItem : D.Decoder SaveItem
decodeSaveItem =
    D.map3 SaveItem
        (D.field "id" D.int)
        (D.field "name" D.string)
        (D.field "savedAt" D.string)


type alias BootData =
    { cardPool : List CardPoolEntry
    , presets : List PresetDeck
    }


decodeBootData : D.Decoder BootData
decodeBootData =
    D.map2 BootData
        (D.field "cardPool" (D.list decodeCardPoolEntry))
        (D.field "presets" (D.list decodePresetDeck))


{-| 11-field decoder via the standard applicative `andMap` pattern,
since elm/json's stock `map*` family stops at 8. `required` reads a
plain field; `optional` wraps in `D.maybe (D.field …)` so a missing
or null field decodes to `Nothing` — used for Creature-only stats
(`power`, `toughness`) and Spell-only `timing`.
-}
decodeCardPoolEntry : D.Decoder CardPoolEntry
decodeCardPoolEntry =
    D.succeed CardPoolEntry
        |> required "id" D.string
        |> required "name" D.string
        |> required "kind" D.string
        |> required "cost_text" D.string
        |> required "colors" (D.list D.string)
        |> required "symbols" (D.list D.string)
        |> required "subtypes" (D.list D.string)
        |> optional "power" D.float
        |> optional "toughness" D.float
        |> optional "timing" D.string
        |> required "abilities" (D.list D.string)


decodePresetDeck : D.Decoder PresetDeck
decodePresetDeck =
    D.map3 PresetDeck
        (D.field "id" D.string)
        (D.field "name" D.string)
        (D.field "cards" (D.list D.string))


required : String -> D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
required field aDec fDec =
    D.map2 (\f a -> f a) fDec (D.field field aDec)


optional : String -> D.Decoder a -> D.Decoder (Maybe a -> b) -> D.Decoder b
optional field aDec fDec =
    D.map2 (\f a -> f a) fDec (D.maybe (D.field field aDec))


optionalField : String -> D.Decoder a -> D.Decoder (Maybe a)
optionalField field decoder =
    D.maybe (D.field field decoder)


{-| iid format is `A:0001:blue-monkey` — the card identifier is the
suffix after the final colon. Mirrors the JS `String(iid).split(':').pop()`.
-}
cardSuffixFromIid : String -> String
cardSuffixFromIid iid =
    case List.reverse (String.split ":" iid) of
        last :: _ ->
            last

        [] ->
            iid



-- AGGREGATION


type alias GameBucket =
    { prompts : List PromptDetails
    , summary : Maybe SummaryDetails
    }


emptyBucket : GameBucket
emptyBucket =
    { prompts = [], summary = Nothing }


bucketByGame : List DecisionRecord -> Dict String GameBucket
bucketByGame records =
    List.foldl addToBucket Dict.empty records


addToBucket : DecisionRecord -> Dict String GameBucket -> Dict String GameBucket
addToBucket rec dict =
    case rec of
        PromptRec details ->
            Dict.update details.gameId
                (\maybeBucket ->
                    let
                        b =
                            Maybe.withDefault emptyBucket maybeBucket
                    in
                    Just { b | prompts = b.prompts ++ [ details ] }
                )
                dict

        SummaryRec details ->
            Dict.update details.gameId
                (\maybeBucket ->
                    let
                        b =
                            Maybe.withDefault emptyBucket maybeBucket
                    in
                    Just { b | summary = Just details }
                )
                dict

        UnknownRec ->
            dict


type alias PerCardCounters =
    { uctRecommended : Int
    , humanPicked : Int
    , wins : Int
    , decidedGames : Int
    }


emptyCounters : PerCardCounters
emptyCounters =
    { uctRecommended = 0, humanPicked = 0, wins = 0, decidedGames = 0 }


type alias Acc =
    { nGames : Int
    , nGamesWithSummary : Int
    , nPrompts : Int
    , nUctPrompts : Int
    , nAgree : Int
    , nDisagree : Int
    , agreeWins : Int
    , agreeLosses : Int
    , disagreeWins : Int
    , disagreeLosses : Int
    , perCard : Dict String PerCardCounters
    }


emptyAcc : Acc
emptyAcc =
    { nGames = 0
    , nGamesWithSummary = 0
    , nPrompts = 0
    , nUctPrompts = 0
    , nAgree = 0
    , nDisagree = 0
    , agreeWins = 0
    , agreeLosses = 0
    , disagreeWins = 0
    , disagreeLosses = 0
    , perCard = Dict.empty
    }


aggregate : List DecisionRecord -> DecisionAggregation
aggregate records =
    let
        total =
            List.length records

        buckets =
            bucketByGame records

        acc =
            Dict.foldl foldBucket emptyAcc buckets
    in
    { totalRecords = total
    , nGames = acc.nGames
    , nGamesWithSummary = acc.nGamesWithSummary
    , nPrompts = acc.nPrompts
    , nUctPrompts = acc.nUctPrompts
    , nAgree = acc.nAgree
    , nDisagree = acc.nDisagree
    , agreeWins = acc.agreeWins
    , agreeLosses = acc.agreeLosses
    , disagreeWins = acc.disagreeWins
    , disagreeLosses = acc.disagreeLosses
    , perCard = sortedPerCard acc.perCard
    }


foldBucket : String -> GameBucket -> Acc -> Acc
foldBucket _ bucket acc0 =
    let
        winner =
            Maybe.andThen .winner bucket.summary

        humanWon =
            winner == Just "A"

        decided =
            winner /= Nothing

        hasSummary =
            case bucket.summary of
                Just _ ->
                    True

                Nothing ->
                    False

        accGameCount =
            { acc0
                | nGames = acc0.nGames + 1
                , nGamesWithSummary = acc0.nGamesWithSummary + boolToInt hasSummary
            }
    in
    List.foldl (foldPrompt humanWon decided) accGameCount bucket.prompts


foldPrompt : Bool -> Bool -> PromptDetails -> Acc -> Acc
foldPrompt humanWon decided prompt acc =
    let
        acc1 =
            { acc | nPrompts = acc.nPrompts + 1 }

        acc2 =
            if prompt.hasUct then
                { acc1 | nUctPrompts = acc1.nUctPrompts + 1 }

            else
                acc1

        acc3 =
            case ( prompt.hasUct, prompt.agreement ) of
                ( True, Just True ) ->
                    { acc2
                        | nAgree = acc2.nAgree + 1
                        , agreeWins = acc2.agreeWins + boolToInt (decided && humanWon)
                        , agreeLosses = acc2.agreeLosses + boolToInt (decided && not humanWon)
                    }

                ( True, Just False ) ->
                    { acc2
                        | nDisagree = acc2.nDisagree + 1
                        , disagreeWins = acc2.disagreeWins + boolToInt (decided && humanWon)
                        , disagreeLosses = acc2.disagreeLosses + boolToInt (decided && not humanWon)
                    }

                _ ->
                    acc2

        acc4 =
            case prompt.uctChosen of
                Just card ->
                    { acc3 | perCard = bumpUctRecommended card acc3.perCard }

                Nothing ->
                    acc3

        acc5 =
            case prompt.humanCard of
                Just card ->
                    let
                        afterHuman =
                            bumpHumanPicked card acc4.perCard

                        afterDecided =
                            if decided then
                                bumpDecided card humanWon afterHuman

                            else
                                afterHuman
                    in
                    { acc4 | perCard = afterDecided }

                Nothing ->
                    acc4
    in
    acc5


boolToInt : Bool -> Int
boolToInt b =
    if b then
        1

    else
        0


bumpUctRecommended : String -> Dict String PerCardCounters -> Dict String PerCardCounters
bumpUctRecommended card dict =
    Dict.update card
        (\maybeRow ->
            let
                row =
                    Maybe.withDefault emptyCounters maybeRow
            in
            Just { row | uctRecommended = row.uctRecommended + 1 }
        )
        dict


bumpHumanPicked : String -> Dict String PerCardCounters -> Dict String PerCardCounters
bumpHumanPicked card dict =
    Dict.update card
        (\maybeRow ->
            let
                row =
                    Maybe.withDefault emptyCounters maybeRow
            in
            Just { row | humanPicked = row.humanPicked + 1 }
        )
        dict


bumpDecided : String -> Bool -> Dict String PerCardCounters -> Dict String PerCardCounters
bumpDecided card humanWon dict =
    Dict.update card
        (\maybeRow ->
            let
                row =
                    Maybe.withDefault emptyCounters maybeRow
            in
            Just
                { row
                    | decidedGames = row.decidedGames + 1
                    , wins =
                        row.wins
                            + (if humanWon then
                                1

                               else
                                0
                              )
                }
        )
        dict


sortedPerCard : Dict String PerCardCounters -> List PerCardRow
sortedPerCard dict =
    Dict.toList dict
        |> List.map
            (\( card, c ) ->
                { card = card
                , uctRecommended = c.uctRecommended
                , humanPicked = c.humanPicked
                , wins = c.wins
                , decidedGames = c.decidedGames
                }
            )
        |> List.sortBy (\r -> negate (r.uctRecommended + r.humanPicked))



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions _ =
    Sub.batch
        [ buildInfoIn BuildInfoReceived
        , logTextIn LogTextReceived
        , logErrorIn LogErrorReceived
        , decisionLogIn DecisionLogReceived
        , savedListIn SavedListReceived
        , saveStatusIn SaveStatusReceived
        , gamePhaseIn GamePhaseReceived
        , bootDataIn BootDataReceived
        ]



-- VIEW


view : Model -> Html Msg
view model =
    div []
        [ viewSaveControls model
        , viewDeckbuilder model
        , viewSavedListPanel model.savedList
        , viewDecisionPanel model.decisionPanel
        , viewLog model.log
        , viewBuildFooter model.build
        ]


viewDeckbuilder : Model -> Html Msg
viewDeckbuilder model =
    if model.gamePhase /= Deckbuilding then
        text ""

    else
        div
            [ id "deckbuilder"
            , style "display" "grid"
            , style "grid-template-columns" "2fr 1fr"
            , style "gap" "1rem"
            ]
            [ viewDeckbuilderPool model
            , viewDeckbuilderDeck model
            ]


viewDeckbuilderPool : Model -> Html Msg
viewDeckbuilderPool model =
    div [ class "pool", style "border" "1px solid #333", style "padding" "0.5rem" ]
        [ Html.h2
            [ style "font-size" "0.7rem"
            , style "margin" "0 0 0.4rem 0"
            , style "color" "#888"
            , style "font-weight" "normal"
            , style "text-transform" "uppercase"
            , style "letter-spacing" "0.05em"
            ]
            [ text ("Card pool (" ++ String.fromInt (List.length model.cardPool) ++ ")") ]
        , viewPoolFilters model
        , viewPoolGrid model
        ]


viewPoolFilters : Model -> Html Msg
viewPoolFilters model =
    let
        allColors =
            model.cardPool
                |> List.concatMap .colors
                |> dedupSorted
    in
    div
        [ class "filters"
        , style "display" "flex"
        , style "gap" "0.5rem"
        , style "margin-bottom" "0.5rem"
        , style "flex-wrap" "wrap"
        ]
        [ deckSelect model.poolFilterColor PoolFilterColorChanged <|
            ( "all colors", "" )
                :: List.map (\c -> ( c, c )) allColors
        , deckSelect model.poolFilterKind PoolFilterKindChanged
            [ ( "all kinds", "" )
            , ( "Creatures", "Creature" )
            , ( "Spells", "Spell" )
            , ( "Artifacts", "Artifact" )
            , ( "Environments", "Environment" )
            , ( "Mutations", "Mutation" )
            ]
        ]


deckSelect : String -> (String -> Msg) -> List ( String, String ) -> Html Msg
deckSelect current toMsg opts =
    Html.select
        [ Html.Events.onInput toMsg
        , style "background" "#1c1c20"
        , style "color" "#ddd"
        , style "border" "1px solid #444"
        , style "padding" "0.2rem 0.4rem"
        , style "font-family" "inherit"
        , style "font-size" "0.7rem"
        ]
        (List.map
            (\( label, value ) ->
                Html.option
                    [ Html.Attributes.value value
                    , Html.Attributes.selected (value == current)
                    ]
                    [ text label ]
            )
            opts
        )


viewPoolGrid : Model -> Html Msg
viewPoolGrid model =
    let
        visible =
            List.filter (poolMatchesFilters model) model.cardPool
    in
    div
        [ class "pool-grid"
        , style "display" "flex"
        , style "flex-wrap" "wrap"
        , style "gap" "0.3rem"
        , style "max-height" "calc(100vh - 16rem)"
        , style "overflow-y" "auto"
        ]
        (List.map viewPoolCard visible)


poolMatchesFilters : Model -> CardPoolEntry -> Bool
poolMatchesFilters model entry =
    let
        colorOk =
            model.poolFilterColor == "" || List.member model.poolFilterColor entry.colors

        kindOk =
            model.poolFilterKind == "" || entry.kind == model.poolFilterKind
    in
    colorOk && kindOk


viewPoolCard : CardPoolEntry -> Html Msg
viewPoolCard entry =
    div
        [ class "card pool-card"
        , Html.Attributes.title entry.id
        , onClick (PoolCardClicked entry.id)
        ]
        ([ div [ class "head" ]
            [ span [ class "name" ] [ text entry.name ]
            , span [ class "cost" ] [ text entry.costText ]
            ]
         ]
            ++ viewCardMetaLine entry
            ++ viewCardAbilities entry.abilities
        )


viewCardMetaLine : CardPoolEntry -> List (Html Msg)
viewCardMetaLine entry =
    let
        statsHtml =
            case ( entry.kind, entry.power, entry.toughness ) of
                ( "Creature", Just p, Just t ) ->
                    [ span [ class "stats" ]
                        [ text (formatStat p ++ "/" ++ formatStat t) ]
                    ]

                _ ->
                    []

        colorTags =
            List.map colorTag entry.colors

        symTags =
            List.map symbolTag entry.symbols

        subtypeSpan =
            if List.isEmpty entry.subtypes then
                []

            else
                [ span [ style "color" "#888" ]
                    [ text (String.join "·" entry.subtypes) ]
                ]

        parts =
            statsHtml ++ colorTags ++ symTags ++ subtypeSpan
    in
    if List.isEmpty parts then
        []

    else
        [ div
            [ class "meta-line"
            , style "display" "flex"
            , style "gap" "0.4rem"
            , style "flex-wrap" "wrap"
            ]
            parts
        ]


formatStat : Float -> String
formatStat f =
    if toFloat (round f) == f then
        String.fromInt (round f)

    else
        String.fromFloat f


colorTag : String -> Html Msg
colorTag c =
    span [ class ("color-" ++ String.left 1 (String.toLower c)) ] [ text c ]


symbolTag : String -> Html Msg
symbolTag s =
    span [ class "symbol" ] [ text s ]


viewCardAbilities : List String -> List (Html Msg)
viewCardAbilities abilities =
    if List.isEmpty abilities then
        []

    else
        [ div [ class "abilities" ]
            (List.map (\s -> div [] [ text s ]) abilities)
        ]


viewDeckbuilderDeck : Model -> Html Msg
viewDeckbuilderDeck model =
    div [ class "deck", style "border" "1px solid #333", style "padding" "0.5rem" ]
        ([ Html.h2
            [ style "font-size" "0.7rem"
            , style "margin" "0 0 0.4rem 0"
            , style "color" "#888"
            , style "font-weight" "normal"
            , style "text-transform" "uppercase"
            , style "letter-spacing" "0.05em"
            ]
            [ text "Your deck" ]
         , div
            [ class "deck-summary"
            , style "color" "#888"
            , style "font-size" "0.7rem"
            , style "margin-bottom" "0.4rem"
            ]
            [ text (String.fromInt (List.length model.deck) ++ " cards") ]
         , viewDeckControls model
         , viewDeckRows model
         , viewStartButton model
         , viewSpectateBlock model
         ]
        )


viewDeckControls : Model -> Html Msg
viewDeckControls model =
    div
        [ class "deck-controls"
        , style "margin-top" "0.5rem"
        , style "display" "flex"
        , style "flex-direction" "column"
        , style "gap" "0.4rem"
        ]
        [ Html.label
            [ style "color" "#888"
            , style "font-size" "0.7rem"
            , style "display" "flex"
            , style "align-items" "center"
            , style "gap" "0.4rem"
            ]
            [ text "Load preset:"
            , deckSelect "" PresetChosen <|
                ( "(choose\u{2026})", "" )
                    :: List.map (\p -> ( p.name, p.id )) model.presets
            ]
        , button [ class "danger", onClick DeckClearClicked ] [ text "Clear deck" ]
        , Html.label
            [ style "color" "#888"
            , style "font-size" "0.7rem"
            , style "display" "flex"
            , style "align-items" "center"
            , style "gap" "0.4rem"
            ]
            [ text "Opponent AI:"
            , deckSelect model.oppAi OppAiChanged
                [ ( "Heuristic (fast)", "heuristic" )
                , ( "UCT (search)", "uct" )
                , ( "MCTS (search)", "mcts" )
                ]
            ]
        ]


viewDeckRows : Model -> Html Msg
viewDeckRows model =
    let
        counts =
            countById model.deck

        nameOf id =
            model.cardPool
                |> List.filter (\e -> e.id == id)
                |> List.head
                |> Maybe.map .name
                |> Maybe.withDefault id

        sorted =
            counts
                |> List.sortWith
                    (\( ida, qa ) ( idb, qb ) ->
                        if qa /= qb then
                            compare qb qa

                        else
                            compare (nameOf ida) (nameOf idb)
                    )
    in
    div [] (List.map (\( id, qty ) -> viewDeckRow (nameOf id) id qty) sorted)


viewDeckRow : String -> String -> Int -> Html Msg
viewDeckRow displayName cardId qty =
    div
        [ class "deck-row"
        , style "display" "flex"
        , style "align-items" "center"
        , style "justify-content" "space-between"
        , style "padding" "0.2rem 0.4rem"
        , style "border-bottom" "1px solid #222"
        , style "font-size" "0.75rem"
        ]
        [ span [ class "qty", style "color" "#fc6", style "min-width" "2.5rem" ]
            [ text (String.fromInt qty ++ "\u{00D7}") ]
        , span
            [ class "name"
            , style "color" "#eee"
            , style "flex" "1"
            , style "margin-left" "0.4rem"
            ]
            [ text displayName ]
        , button
            [ onClick (DeckRowRemove cardId)
            , style "padding" "0.1rem 0.4rem"
            , style "font-size" "0.7rem"
            ]
            [ text "-" ]
        ]


countById : List String -> List ( String, Int )
countById ids =
    ids
        |> List.foldl
            (\id dict ->
                Dict.update id (\m -> Just (Maybe.withDefault 0 m + 1)) dict
            )
            Dict.empty
        |> Dict.toList


viewStartButton : Model -> Html Msg
viewStartButton model =
    button
        [ class "start"
        , onClick StartGameClicked
        , Html.Attributes.disabled (List.isEmpty model.deck)
        , style "margin-top" "0.8rem"
        , style "padding" "0.5rem 1rem"
        , style "font-size" "0.85rem"
        , style "border-color" "#6f9"
        ]
        [ text "Start game" ]


viewSpectateBlock : Model -> Html Msg
viewSpectateBlock model =
    div
        [ style "margin-top" "1.2rem"
        , style "padding-top" "0.6rem"
        , style "border-top" "1px solid #333"
        , style "display" "flex"
        , style "flex-direction" "column"
        , style "gap" "0.4rem"
        ]
        [ div [ style "color" "#888", style "font-size" "0.7rem" ]
            [ text "Spectate (both AI, watch with scrubber)" ]
        , aiPickerLabel "A:" model.specAiA SpecAiAChanged
        , aiPickerLabel "B:" model.specAiB SpecAiBChanged
        , button
            [ class "start"
            , onClick StartSpectateClicked
            , Html.Attributes.disabled (List.isEmpty model.deck)
            , style "border-color" "#cf6"
            , style "padding" "0.5rem 1rem"
            , style "font-size" "0.85rem"
            ]
            [ text "Spectate (both AI)" ]
        ]


aiPickerLabel : String -> String -> (String -> Msg) -> Html Msg
aiPickerLabel labelText current toMsg =
    Html.label
        [ style "color" "#888"
        , style "font-size" "0.7rem"
        , style "display" "flex"
        , style "align-items" "center"
        , style "gap" "0.4rem"
        ]
        [ text labelText
        , deckSelect current toMsg
            [ ( "Heuristic", "heuristic" )
            , ( "UCT", "uct" )
            , ( "MCTS", "mcts" )
            ]
        ]


dedupSorted : List String -> List String
dedupSorted xs =
    xs
        |> List.foldl
            (\x acc ->
                if List.member x acc then
                    acc

                else
                    x :: acc
            )
            []
        |> List.sort


viewSaveControls : Model -> Html Msg
viewSaveControls model =
    let
        playing =
            model.gamePhase == Playing
    in
    div
        [ id "save-controls"
        , style "margin-bottom" "0.5rem"
        , style "display" "flex"
        , style "gap" "0.4rem"
        , style "flex-wrap" "wrap"
        , style "align-items" "center"
        ]
        [ button
            [ onClick SaveClicked
            , Html.Attributes.disabled (not playing)
            ]
            [ text "Save" ]
        , button [ onClick SavedListToggleClicked ] [ text "Load saved\u{2026}" ]
        , button
            [ onClick DownloadClicked
            , Html.Attributes.disabled (not playing)
            ]
            [ text "Download" ]
        , button [ onClick LoadFromFileClicked ] [ text "Load file\u{2026}" ]
        , button
            [ onClick DecisionReportClicked
            , Html.Attributes.title "Inline decision report — UCT-vs-human stats from all played games"
            ]
            [ text "Decision report" ]
        , button
            [ onClick DecisionExportClicked
            , Html.Attributes.title "Export decision log as JSONL (for the Python aggregator)"
            ]
            [ text "Export" ]
        , button
            [ onClick DecisionClearClicked
            , class "danger"
            , Html.Attributes.title "Delete all recorded decision-log records from IndexedDB"
            ]
            [ text "Clear" ]
        , button
            [ onClick TestPanicClicked
            , class "danger"
            , style "margin-left" "auto"
            ]
            [ text "Trigger test panic" ]
        , span
            [ id "save-status"
            , style "color" "#888"
            , style "font-size" "0.7rem"
            ]
            [ text model.saveStatus ]
        ]


viewLog : List LogEntry -> Html Msg
viewLog entries =
    pre
        [ id logContainerId
        , style "max-height" "24rem"
        , style "overflow-y" "auto"
        , style "font-size" "0.75rem"
        , style "color" "#aaa"
        , style "white-space" "pre"
        , style "margin" "0"
        ]
        (List.map viewEntry entries)


viewEntry : LogEntry -> Html Msg
viewEntry entry =
    case entry of
        TextLine line ->
            text (line ++ "\n")

        ErrorEntry ev ->
            viewErrorBlock ev


viewErrorBlock : ErrorEvent -> Html Msg
viewErrorBlock ev =
    div [ class "log-error" ]
        ([ div [ class "log-error-header" ]
            [ text ("[" ++ String.toUpper ev.source ++ "] " ++ ev.message) ]
         , div [ class "log-error-meta" ]
            [ text (formatErrorMeta ev) ]
         ]
            ++ viewBreadcrumb ev.breadcrumb
            ++ viewJsStack ev.jsStack
            ++ viewRawStderr ev.rawStderr
            ++ viewAbortFooter ev.source
        )


formatErrorMeta : ErrorEvent -> String
formatErrorMeta ev =
    let
        parts =
            List.filterMap identity
                [ Maybe.map (\l -> "at " ++ l) ev.location
                , Maybe.map (\c -> "inside FFI " ++ c) ev.ffiCall
                , Maybe.map (\us -> "t=" ++ formatMillis us ++ "ms") ev.atUs
                ]
    in
    String.join "  ·  " parts


formatMillis : Float -> String
formatMillis us =
    let
        ms =
            us / 1000

        rounded =
            toFloat (round (ms * 10)) / 10
    in
    String.fromFloat rounded


viewBreadcrumb : List String -> List (Html Msg)
viewBreadcrumb crumbs =
    if List.isEmpty crumbs then
        []

    else
        div [ class "log-error-trail" ]
            [ text
                ("--- last "
                    ++ String.fromInt (List.length crumbs)
                    ++ " trace events before failure ---"
                )
            ]
            :: List.map
                (\line -> div [ class "log-error-trail-line" ] [ text line ])
                crumbs


viewJsStack : Maybe String -> List (Html Msg)
viewJsStack maybeStack =
    case maybeStack of
        Nothing ->
            []

        Just stack ->
            [ div [ class "log-error-trail" ] [ text "--- JS exception stack ---" ]
            , div
                [ class "log-error-trail-line"
                , style "white-space" "pre-wrap"
                ]
                [ text stack ]
            ]


viewRawStderr : Maybe String -> List (Html Msg)
viewRawStderr maybeStderr =
    case maybeStderr of
        Nothing ->
            []

        Just stderrText ->
            [ div [ class "log-error-trail" ] [ text "--- raw stderr from wasm ---" ]
            , div
                [ class "log-error-trail-line"
                , style "white-space" "pre-wrap"
                ]
                [ text stderrText ]
            ]


viewAbortFooter : String -> List (Html Msg)
viewAbortFooter source =
    if source == "rust-panic" || source == "wasm-trap" then
        [ div [ class "log-error-meta" ]
            [ text "wasm module aborted after this point — reload the page to continue" ]
        ]

    else
        []


viewBuildFooter : BuildState -> Html Msg
viewBuildFooter state =
    case state of
        AwaitingPort ->
            text ""

        NoBuildInfo ->
            footerDiv [ text "tsot · build info unavailable" ]

        HasBuildInfo info ->
            footerDiv
                [ text
                    ("tsot · "
                        ++ info.profile
                        ++ " · built "
                        ++ info.builtAt
                        ++ " · "
                        ++ info.commit
                    )
                ]


footerDiv : List (Html msg) -> Html msg
footerDiv children =
    div
        [ style "position" "fixed"
        , style "bottom" "0"
        , style "right" "0"
        , style "padding" "0.15rem 0.5rem"
        , style "background" "rgba(20,20,28,0.85)"
        , style "border-top-left-radius" "4px"
        , style "color" "#555"
        , style "font-size" "0.65rem"
        , style "font-family" "ui-monospace, SFMono-Regular, Menlo, monospace"
        , style "pointer-events" "none"
        , style "z-index" "1000"
        ]
        children


viewSavedListPanel : SavedListState -> Html Msg
viewSavedListPanel state =
    case state of
        SavedHidden ->
            text ""

        SavedLoading ->
            savedListDiv [ text "Loading saves…" ]

        SavedShown [] ->
            savedListDiv
                [ text "(no saves yet)"
                , savedListCloseButton
                ]

        SavedShown items ->
            savedListDiv
                (List.map viewSaveRow items
                    ++ [ savedListCloseButton ]
                )

        SavedError err ->
            savedListDiv
                [ div [ style "color" "#f88" ]
                    [ text ("Failed to read IndexedDB: " ++ err) ]
                , savedListCloseButton
                ]


savedListDiv : List (Html Msg) -> Html Msg
savedListDiv children =
    div
        [ id "saved-list"
        , style "border" "1px solid #333"
        , style "padding" "0.4rem"
        , style "margin-bottom" "0.5rem"
        ]
        children


savedListCloseButton : Html Msg
savedListCloseButton =
    div [ style "margin-top" "0.4rem" ]
        [ button [ onClick SavedListClosed ] [ text "Close" ] ]


viewSaveRow : SaveItem -> Html Msg
viewSaveRow item =
    div
        [ style "display" "flex"
        , style "gap" "0.4rem"
        , style "align-items" "center"
        , style "padding" "0.2rem 0"
        ]
        [ span [ style "flex" "1" ]
            [ text item.name
            , span [ style "color" "#666" ] [ text (" — " ++ item.savedAt) ]
            ]
        , button [ onClick (SavedItemLoad item.id) ] [ text "Load" ]
        , button [ onClick (SavedItemDownload item.id) ] [ text "Download" ]
        , button
            [ class "danger"
            , onClick (SavedItemDelete item.id)
            ]
            [ text "Delete" ]
        ]


viewDecisionPanel : DecisionPanel -> Html Msg
viewDecisionPanel panel =
    case panel of
        DecisionHidden ->
            text ""

        DecisionLoading ->
            decisionPanelDiv [ text "Loading decision log…" ]

        DecisionShown agg ->
            decisionPanelDiv (viewDecisionReport agg)

        DecisionError err ->
            decisionPanelDiv
                [ div [ style "color" "#f88" ]
                    [ text ("Failed to decode decision log: " ++ err) ]
                , decisionCloseButton
                ]


decisionPanelDiv : List (Html Msg) -> Html Msg
decisionPanelDiv children =
    div
        [ id "decision-report"
        , style "border" "1px solid #333"
        , style "padding" "0.6rem"
        , style "margin-bottom" "0.5rem"
        , style "font-size" "0.75rem"
        ]
        children


decisionCloseButton : Html Msg
decisionCloseButton =
    div [ style "margin-top" "0.4rem" ]
        [ button [ onClick DecisionPanelClosed ] [ text "Close" ] ]


viewDecisionReport : DecisionAggregation -> List (Html Msg)
viewDecisionReport agg =
    if agg.totalRecords == 0 then
        [ div [ style "color" "#888" ]
            [ text "No decisions recorded yet — play a game to populate." ]
        , decisionCloseButton
        ]

    else
        [ viewDecisionHeader agg
        , viewDecisionStatsGrid agg
        , viewPerCardTable agg.perCard
        , decisionCloseButton
        ]


viewDecisionHeader : DecisionAggregation -> Html Msg
viewDecisionHeader agg =
    div
        [ style "display" "flex"
        , style "justify-content" "space-between"
        , style "align-items" "baseline"
        ]
        [ h2
            [ style "font-size" "0.8rem"
            , style "margin" "0 0 0.4rem"
            , style "color" "#6cf"
            , style "font-weight" "normal"
            ]
            [ text "tsot · decision report" ]
        , span [ style "color" "#666", style "font-size" "0.65rem" ]
            [ text
                (String.fromInt agg.totalRecords
                    ++ " record(s) · "
                    ++ String.fromInt agg.nGames
                    ++ " game(s)"
                )
            ]
        ]


viewDecisionStatsGrid : DecisionAggregation -> Html Msg
viewDecisionStatsGrid agg =
    div
        [ style "display" "grid"
        , style "grid-template-columns" "auto auto"
        , style "gap" "0.4rem 1.5rem"
        , style "margin-bottom" "0.6rem"
        ]
        (statsRow "games (any data)" (String.fromInt agg.nGames)
            ++ statsRow "games with recorded winner" (String.fromInt agg.nGamesWithSummary)
            ++ statsRow "prompts logged" (String.fromInt agg.nPrompts)
            ++ statsRow "prompts with UCT belief" (String.fromInt agg.nUctPrompts)
            ++ statsRow "UCT-human agreed / disagreed"
                (String.fromInt agg.nAgree ++ " / " ++ String.fromInt agg.nDisagree)
            ++ statsRow "win-rate when human agreed with UCT"
                (formatPct agg.agreeWins (agg.agreeWins + agg.agreeLosses))
            ++ statsRow "win-rate when human disagreed with UCT"
                (formatPct agg.disagreeWins (agg.disagreeWins + agg.disagreeLosses))
        )


statsRow : String -> String -> List (Html Msg)
statsRow label value =
    [ span [] [ text label ]
    , span [ style "color" "#fc6" ] [ text value ]
    ]


formatPct : Int -> Int -> String
formatPct n d =
    if d == 0 then
        "—"

    else
        let
            ratio =
                100 * toFloat n / toFloat d

            rounded =
                toFloat (round (ratio * 10)) / 10
        in
        String.fromFloat rounded ++ "%"


viewPerCardTable : List PerCardRow -> Html Msg
viewPerCardTable rows =
    table [ style "border-collapse" "collapse", style "width" "100%" ]
        (perCardHeaderRow :: List.map viewPerCardRow rows)


perCardHeaderRow : Html Msg
perCardHeaderRow =
    tr [ style "color" "#888" ]
        [ headerCell "card"
        , headerCell "UCT chose"
        , headerCell "human picked"
        , headerCell "wins/games (human picked)"
        , headerCell "win-rate"
        ]


headerCell : String -> Html Msg
headerCell label =
    th
        [ style "text-align" "left"
        , style "border" "1px solid #333"
        , style "padding" "0.2rem 0.4rem"
        ]
        [ text label ]


viewPerCardRow : PerCardRow -> Html Msg
viewPerCardRow row =
    tr []
        [ td [ style "padding" "0.2rem 0.4rem" ] [ text row.card ]
        , td [ style "padding" "0.2rem 0.4rem" ] [ text (String.fromInt row.uctRecommended) ]
        , td [ style "padding" "0.2rem 0.4rem" ] [ text (String.fromInt row.humanPicked) ]
        , td [ style "padding" "0.2rem 0.4rem" ]
            [ text (String.fromInt row.wins ++ "/" ++ String.fromInt row.decidedGames) ]
        , td [ style "padding" "0.2rem 0.4rem" ]
            [ text (formatPct row.wins row.decidedGames) ]
        ]



-- MAIN


main : Program () Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        }
