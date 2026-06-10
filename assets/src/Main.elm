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
import BuildFooter
import Card
import Error
import Dict exposing (Dict)
import GameScreen
import Set
import Html exposing (Html, button, div, h2, pre, span, table, td, text, th, tr)
import Html.Attributes exposing (class, id, style)
import Html.Events exposing (onClick)
import Html.Keyed as Keyed
import Json.Decode as D
import Json.Encode as E
import LogPanel
import SpectatorBar
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


{-| Stage 11a — game-screen meta line (`turn N · phase X · active A ·
you are B`). play.html's `_renderInner` pushes the four fields here
after each FFI envelope; the rest of the game-screen DOM stays
JS-rendered until later 11 substages.
-}
port gameMetaIn : (D.Value -> msg) -> Sub msg


{-| Stage 11c — the `#prompt` line at the top of the page. Set from
~12 sites in play.html (game state via `_renderInner`, bootstrap
stage markers + watchdog, error paths in onSaveClick / loadSaveJson /
startGameFromDeckbuilder / startSpectate). Carries a plain String;
the previous styled-span variant of the Spectate prompt loses its
color until prompts gain a richer envelope.
-}
port promptTextIn : (String -> msg) -> Sub msg


{-| Stage 11d — the full `{state, prompt}` envelope that _renderInner
receives on every render. Stored raw as `D.Value` for now; subsequent
11 substages add decoders + view functions for specific slices (board,
hand, deck-top, buttons, prompt variants) as they need them. No
visible render today — the win is architectural: state arrives in
Elm, available to every future view function without adding another
port per feature.
-}
port gameStateIn : (D.Value -> msg) -> Sub msg


{-| Stage 12 — spectator bar state push. JS-side `state.spectate`
(snapshots + index + interval handle + speed) is the source of truth;
this port carries the projection Elm needs to render the bar (active,
index, total, playing, msPerStep, winner, current snapshot's
turn/phase/activePlayer). Pushed on every spectator state change:
seek / step / play tick / pause / speed change / exit.
-}
port spectatorStateIn : (D.Value -> msg) -> Sub msg


{-| Chunk B/C Wave 0 — UCT preview push from the worker. JS owns the
preview kickoff (cancellation + worker round-trip + stale-promise
guard), but the result envelope `{candidates, chosen,
iterations_completed, prompt_key, in_flight}` is pushed here so Elm
can decorate `viewCard` with badges + the recommended-pick outline.
Wired in Wave 0 but consumed by Wave 5 (UCT preview + casting banner).
-}
port uctPreviewIn : (D.Value -> msg) -> Sub msg


{-| One outbound port for every IDB-bound action. Carries an
`{op, payload}` envelope. Per-feature ports collapsed here:
`decision_get_all` / `decision_export` / `decision_clear` /
`saved_get_all` / `saved_item_action` (the last carries
`{action, id}` in its payload).
-}
port idbReqOut : { op : String, payload : E.Value } -> Cmd msg



-- MODEL


-- BuildInfo / BuildState moved to BuildFooter.elm
-- ErrorEvent / LogEntry moved to LogPanel.elm


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


type alias GameMeta =
    { turn : Int
    , phase : String
    , activePlayer : String
    , viewer : String
    }


{-| Counts + cards + deck-top back for one player, decoded out of
`Model.gameState.state.players[i]`. Hand-count is opponent-only in the
UI (the viewer sees their own hand directly). Card lists default to
empty when absent (opp.hand is filtered server-side, etc.).
-}
type alias PlayerCounts =
    { side : String
    , board : List Card.Card
    , hand : List Card.Card
    , graveyard : List Card.Card
    , deckCount : Int
    , handCount : Int
    , exileCount : Int
    , graveyardCount : Int
    , deckTop : Maybe Card.Card
    }


-- DeckBack type consolidated into Card.Card 2026-06-09. The deck-top
-- widget renders via Card.view Card.Back (symbols-only, no color
-- per C.1).


type alias GameViewSlice =
    { viewer : String
    , you : PlayerCounts
    , opp : PlayerCounts
    }


type alias Model =
    { build : BuildFooter.State
    , log : List LogPanel.Entry
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
    , gameMeta : Maybe GameMeta
    , promptText : String
    , gameState : Maybe D.Value
    , spectatorBar : SpectatorBar.Model
    , prompt : GameScreen.Prompt
    , chooseIntDraft : String
    , uctPreview : Maybe GameScreen.UctPreview
    , combat : GameScreen.CombatSelection
    , actionInFlight : Bool
    , errors : List Error.Error
    , nextErrorId : Int
    }


-- logContainerId moved to LogPanel.containerId



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
    | GameMetaReceived D.Value
    | PromptTextReceived String
    | GameStateReceived D.Value
    | SpectatorStateReceived D.Value
    | SpecBackEndClicked
    | SpecStepBackClicked
    | SpecPlayPauseClicked
    | SpecStepFwdClicked
    | SpecFwdEndClicked
    | SpecSliderChanged String
    | SpecSpeedChanged String
    | SpecExitClicked
    | UctPreviewReceived D.Value
    | ConfirmYesClicked
    | ConfirmNoClicked
    | PlayerChoiceClicked (Maybe String)
    | IntChoiceInputChanged String
    | IntChoiceConfirmClicked Int
    | HandCardClicked String
    | BoardActivationClicked GameScreen.Activation
    | PassClicked
    | TargetCardClicked String
    | SkipChoiceCardClicked
    | AttackerToggled String
    | BlockerClicked String
    | AttackerTargetedForBlock String
    | ConfirmAttackersClicked
    | NoAttackClicked
    | ConfirmBlocksClicked
    | NoBlocksClicked
    | NoOp



-- INIT


init : () -> ( Model, Cmd Msg )
init _ =
    ( { build = BuildFooter.AwaitingPort
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
      , gameMeta = Nothing
      , promptText = "Loading\u{2026}"
      , gameState = Nothing
      , spectatorBar = SpectatorBar.init
      , prompt = GameScreen.LoadingPrompt
      , chooseIntDraft = ""
      , uctPreview = Nothing
      , combat = GameScreen.emptyCombatSelection
      , actionInFlight = False
      , errors = []
      , nextErrorId = 0
      }
    , Cmd.none
    )


{-| Convert a decode failure into a typed `Error.Error` and append it
to `Model.errors`. The single canonical path for "a Port payload failed
to decode" — per ERROR.md axiom no decode failure may be silently
dropped (the failing payload either reaches the developer with full
context, or the bug hides until something else surfaces it).

`ctx.surface` + `ctx.region` tell the renderer WHERE to anchor the
overlay (deckbuilder dropdown, prompt bar, etc.); the `title`
summarizes what failed; `D.errorToString err` gives the JSON path +
expected/got. The Error.id is a per-session monotonic counter so
re-renders preserve identity (Html.Keyed groundwork for Slice 6).
-}
pushDecodeError : Error.Context -> String -> D.Error -> Model -> Model
pushDecodeError ctx title decodeErr model =
    let
        newError : Error.Error
        newError =
            { id = "err-" ++ ctx.surface ++ "-" ++ String.fromInt model.nextErrorId
            , severity = Error.LevelError
            , context = ctx
            , title = title
            , why = D.errorToString decodeErr
            , trace = []
            , raw = Nothing
            , at = ""
            }
    in
    { model
        | errors = model.errors ++ [ newError ]
        , nextErrorId = model.nextErrorId + 1
    }


{-| Variant of `pushDecodeError` taking a pre-decoded `Result` and a
surface name. Pushes an Error when the Result is `Err`; no-op when
`Ok`. Use this at sites where a safe fallback ALREADY converts the
Result to a value (`Result.withDefault`, `Result.toMaybe`) but the
underlying failure should still surface — without this helper those
sites used to be the canonical place to silently swallow.
-}
maybePushDecodeError : String -> String -> Result D.Error a -> Model -> Model
maybePushDecodeError surface title result model =
    case result of
        Ok _ ->
            model

        Err err ->
            pushDecodeError
                { surface = surface, region = Nothing, anchor = Nothing }
                title
                err
                model


{-| Render the subset of `Model.errors` that originated at a given
surface, stacked as overlays inside the caller's positioned container.
Per ERROR.md § Visual contract the overlay anchors AT the originating
surface, not in a global LOG drawer — so each surface's render
wraps itself in a `position: relative` container and inserts this
helper to get its own errors anchored locally.

Empty case returns `text ""` so callers can unconditionally splice it
in without conditional logic.
-}
viewErrorsForSurface : String -> List Error.Error -> Html msg
viewErrorsForSurface surface errors =
    let
        matching =
            List.filter (\e -> e.context.surface == surface) errors
    in
    if List.isEmpty matching then
        text ""

    else
        div [ class "tsot-error-stack" ]
            (List.map Error.view matching)


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
            case D.decodeValue BuildFooter.decode value of
                Ok info ->
                    ( { model | build = BuildFooter.HasBuildInfo info }, Cmd.none )

                Err err ->
                    -- Build-info is decorative; if it doesn't decode the
                    -- footer renders "no build info" rather than blocking
                    -- the page. But silently swallowing the decode error
                    -- violates the axiom — surface it so a drifting
                    -- payload shape doesn't hide.
                    ( pushDecodeError
                        { surface = "build-footer", region = Nothing, anchor = Nothing }
                        "buildInfoIn decode failed"
                        err
                        { model | build = BuildFooter.NoBuildInfo }
                    , Cmd.none
                    )

        LogTextReceived line ->
            ( { model | log = model.log ++ [ LogPanel.TextLine line ] }, scrollLogToBottom )

        LogErrorReceived value ->
            case D.decodeValue LogPanel.decodeError value of
                Ok ev ->
                    ( { model | log = model.log ++ [ LogPanel.ErrorEntry ev ] }, scrollLogToBottom )

                Err err ->
                    ( { model
                        | log =
                            model.log
                                ++ [ LogPanel.TextLine ("[log decode failed] " ++ D.errorToString err) ]
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
            -- Diagnostic: surface the preset count the JS bridge actually
            -- delivered, BEFORE decoding the rest. If this lands as "3"
            -- and the dropdown still shows 2, the failure is downstream of
            -- decodeBootData; if it lands as "2" the failure is upstream
            -- (wasm or JS forwarding). The line goes to the LOG drawer
            -- as a TextLine (not the Error overlay) because it's
            -- informational, not an error.
            let
                presetCountFromJson =
                    D.decodeValue
                        (D.field "presets" (D.list (D.succeed ())))
                        value
                        |> Result.map List.length
                        |> Result.withDefault -1

                modelWithDiag =
                    { model
                        | log =
                            model.log
                                ++ [ LogPanel.TextLine
                                        ("bootDataIn: presets array length = "
                                            ++ String.fromInt presetCountFromJson
                                        )
                                   ]
                    }
            in
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
                    ( { modelWithDiag
                        | cardPool = boot.cardPool
                        , presets = boot.presets
                        , deck = deck
                      }
                    , scrollLogToBottom
                    )

                Err err ->
                    -- CLAUDE.md "errors are sacred." Routes through the
                    -- typed Error pipeline; ends up overlay-anchored
                    -- at the deckbuilder surface (Slice 2 of ERROR.md)
                    -- so a new preset failing to decode surfaces inline
                    -- at the dropdown instead of as a silent phantom.
                    ( pushDecodeError
                        { surface = "deckbuilder"
                        , region = Just "preset-dropdown"
                        , anchor = Nothing
                        }
                        "bootDataIn decode failed"
                        err
                        modelWithDiag
                    , scrollLogToBottom
                    )

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
                            , ( "msPerStep", E.int model.spectatorBar.msPerStep )
                            ]
                    }
                )

        GameMetaReceived value ->
            case D.decodeValue decodeGameMeta value of
                Ok meta ->
                    ( { model | gameMeta = Just meta }, Cmd.none )

                Err err ->
                    -- The meta line shows turn/phase/active-player. A
                    -- decode failure means the line stops updating, which
                    -- is silently destabilizing to a developer trying to
                    -- reason about engine state. Surface inline at the
                    -- meta row.
                    ( pushDecodeError
                        { surface = "game-meta", region = Nothing, anchor = Nothing }
                        "gameMetaIn decode failed"
                        err
                        model
                    , Cmd.none
                    )

        PromptTextReceived text ->
            ( { model | promptText = text }, Cmd.none )

        GameStateReceived value ->
            let
                promptResult =
                    D.decodeValue (D.field "prompt" GameScreen.decodePrompt) value

                newPrompt =
                    Result.withDefault GameScreen.LoadingPrompt promptResult

                newDraft =
                    case ( model.prompt, newPrompt ) of
                        ( GameScreen.ChooseIntPrompt _, GameScreen.ChooseIntPrompt _ ) ->
                            model.chooseIntDraft

                        ( _, GameScreen.ChooseIntPrompt data ) ->
                            String.fromInt data.min

                        _ ->
                            ""

                newCombat =
                    if isCombatPrompt newPrompt then
                        model.combat

                    else
                        GameScreen.emptyCombatSelection

                sliceResult =
                    D.decodeValue decodeGameViewSlice value

                maybeSlice =
                    Result.toMaybe sliceResult

                newPromptText =
                    GameScreen.promptToText
                        (Maybe.map (promptCtxFromSlice maybeSlice) maybeSlice)
                        newPrompt

                -- Surface decode failures even though we have safe
                -- fallbacks (LoadingPrompt / no-slice). Per ERROR.md
                -- the developer should see WHY the prompt or slice
                -- didn't decode, instead of staring at "Loading..."
                -- with no explanation.
                modelWithErrors =
                    model
                        |> maybePushDecodeError "prompt"
                            "gameStateIn .prompt decode failed"
                            promptResult
                        |> maybePushDecodeError "game-screen"
                            "gameStateIn .state decode failed"
                            sliceResult
            in
            ( { modelWithErrors
                | gameState = Just value
                , prompt = newPrompt
                , chooseIntDraft = newDraft
                , combat = newCombat
                , promptText = newPromptText
                , actionInFlight = False
              }
            , Cmd.none
            )

        SpectatorStateReceived value ->
            case D.decodeValue SpectatorBar.decode value of
                Ok bar ->
                    ( { model | spectatorBar = bar }, Cmd.none )

                Err err ->
                    -- Spectator bar drops behind the live state if its
                    -- port payload doesn't decode. The scrubber + speed
                    -- + play/pause stop responding silently. Surface
                    -- inline so the operator sees the cause instead of
                    -- a frozen bar.
                    ( pushDecodeError
                        { surface = "spectator-bar", region = Nothing, anchor = Nothing }
                        "spectatorStateIn decode failed"
                        err
                        model
                    , Cmd.none
                    )

        SpecBackEndClicked ->
            ( model, sendSpecCmd "spec_seek" (E.object [ ( "index", E.int 0 ) ]) )

        SpecStepBackClicked ->
            ( model, sendSpecCmd "spec_step" (E.object [ ( "delta", E.int -1 ) ]) )

        SpecPlayPauseClicked ->
            let
                cmd =
                    if model.spectatorBar.playing then
                        "spec_pause"

                    else
                        "spec_play"
            in
            ( model, sendSpecCmd cmd E.null )

        SpecStepFwdClicked ->
            ( model, sendSpecCmd "spec_step" (E.object [ ( "delta", E.int 1 ) ]) )

        SpecFwdEndClicked ->
            ( model, sendSpecCmd "spec_fwd_end" E.null )

        SpecSliderChanged str ->
            case String.toInt str of
                Just i ->
                    ( model, sendSpecCmd "spec_seek" (E.object [ ( "index", E.int i ) ]) )

                Nothing ->
                    ( model, Cmd.none )

        SpecSpeedChanged str ->
            case String.toInt str of
                Just ms ->
                    let
                        bar =
                            model.spectatorBar
                    in
                    ( { model | spectatorBar = { bar | msPerStep = ms } }
                    , sendSpecCmd "spec_set_speed" (E.object [ ( "ms", E.int ms ) ])
                    )

                Nothing ->
                    ( model, Cmd.none )

        SpecExitClicked ->
            ( model, sendSpecCmd "spec_exit" E.null )

        UctPreviewReceived value ->
            let
                uctResult =
                    D.decodeValue GameScreen.decodeUctPreview value
            in
            ( maybePushDecodeError "game-screen"
                "uctPreviewIn decode failed"
                uctResult
                { model | uctPreview = Result.toMaybe uctResult }
            , Cmd.none
            )

        ConfirmYesClicked ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoiceConfirm" )
                    , ( "yes", E.bool True )
                    ]
                )

        ConfirmNoClicked ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoiceConfirm" )
                    , ( "yes", E.bool False )
                    ]
                )

        PlayerChoiceClicked maybePid ->
            let
                playerField =
                    case maybePid of
                        Just pid ->
                            E.string pid

                        Nothing ->
                            E.null
            in
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoicePlayer" )
                    , ( "player", playerField )
                    ]
                )

        IntChoiceInputChanged str ->
            ( { model | chooseIntDraft = str }, Cmd.none )

        IntChoiceConfirmClicked v ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoiceInt" )
                    , ( "value", E.int v )
                    ]
                )

        HandCardClicked iid ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "PlayCard" )
                    , ( "iid", E.string iid )
                    ]
                )

        BoardActivationClicked act ->
            fireActivate model
                (E.object
                    [ ( "iid", E.string act.iid )
                    , ( "ability_index", E.int act.abilityIndex )
                    , ( "needs_x", E.bool act.needsX )
                    , ( "text", E.string act.text )
                    ]
                )

        PassClicked ->
            fireAction model (E.object [ ( "kind", E.string "Pass" ) ])

        TargetCardClicked iid ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoiceCard" )
                    , ( "iid", E.string iid )
                    ]
                )

        SkipChoiceCardClicked ->
            fireAction model
                (E.object
                    [ ( "kind", E.string "ChoiceCard" )
                    , ( "iid", E.null )
                    ]
                )

        AttackerToggled iid ->
            ( { model | combat = GameScreen.toggleAttacker iid model.combat }, Cmd.none )

        BlockerClicked iid ->
            ( { model | combat = GameScreen.clickBlocker iid model.combat }, Cmd.none )

        AttackerTargetedForBlock iid ->
            ( { model | combat = GameScreen.assignAttackerToStaged iid model.combat }, Cmd.none )

        ConfirmAttackersClicked ->
            fireAction { model | combat = GameScreen.emptyCombatSelection }
                (E.object
                    [ ( "kind", E.string "Attackers" )
                    , ( "iids", E.list E.string (Set.toList model.combat.attackers) )
                    ]
                )

        NoAttackClicked ->
            fireAction { model | combat = GameScreen.emptyCombatSelection }
                (E.object
                    [ ( "kind", E.string "Attackers" )
                    , ( "iids", E.list E.string [] )
                    ]
                )

        ConfirmBlocksClicked ->
            fireAction { model | combat = GameScreen.emptyCombatSelection }
                (E.object
                    [ ( "kind", E.string "Blocks" )
                    , ( "pairs", encodeBlocksPairs model.combat.blocks )
                    ]
                )

        NoBlocksClicked ->
            fireAction { model | combat = GameScreen.emptyCombatSelection }
                (E.object
                    [ ( "kind", E.string "Blocks" )
                    , ( "pairs", E.list identity [] )
                    ]
                )

        NoOp ->
            ( model, Cmd.none )


{-| The engine accepts pairs as `[[blockerIid, attackerIid], ...]`.
-}
encodeBlocksPairs : Dict String String -> E.Value
encodeBlocksPairs blocks =
    blocks
        |> Dict.toList
        |> E.list
            (\( blkIid, atkIid ) ->
                E.list E.string [ blkIid, atkIid ]
            )


isCombatPrompt : GameScreen.Prompt -> Bool
isCombatPrompt p =
    case p of
        GameScreen.PickAttackersPrompt _ ->
            True

        GameScreen.PickBlocksPrompt _ ->
            True

        _ ->
            False


{-| Bridges the chunk-A `GameViewSlice` (viewer + you/opp records) into
the `GameScreen.promptToText` context shape (viewer + iid→name
function). `labelByIid` searches every visible zone of both players
for a matching iid and returns the card name; missing iids fall back
to the raw iid (a "should never happen" path).
-}
promptCtxFromSlice : Maybe GameViewSlice -> GameViewSlice -> { viewer : String, labelByIid : String -> String }
promptCtxFromSlice _ slice =
    { viewer = slice.viewer
    , labelByIid = labelForIid (Just slice)
    }


applyAction : E.Value -> Cmd Msg
applyAction action =
    workerCmdOut { cmd = "apply_action", payload = action }


{-| Fire an action with an in-flight guard. If a previous action's
response hasn't landed yet (model.actionInFlight = True), the new
action is silently dropped — prevents the double-click-pass during
PickCard from queuing two Pass actions, the second of which the
engine receives during DeclareBlockers and panics on. Cleared in
GameStateReceived when the next state envelope arrives.
-}
fireAction : Model -> E.Value -> ( Model, Cmd Msg )
fireAction model action =
    if model.actionInFlight then
        ( model, Cmd.none )

    else
        ( { model | actionInFlight = True }
        , workerCmdOut { cmd = "apply_action", payload = action }
        )


fireActivate : Model -> E.Value -> ( Model, Cmd Msg )
fireActivate model payload =
    if model.actionInFlight then
        ( model, Cmd.none )

    else
        ( { model | actionInFlight = True }
        , workerCmdOut { cmd = "activate_ability", payload = payload }
        )


sendSpecCmd : String -> E.Value -> Cmd Msg
sendSpecCmd cmd payload =
    workerCmdOut { cmd = cmd, payload = payload }


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
    Browser.Dom.setViewportOf LogPanel.containerId 0 1000000
        |> Task.attempt (\_ -> NoOp)



-- DECODERS


-- decodeBuildInfo moved to BuildFooter.decode
-- decodeErrorEvent moved to LogPanel.decodeError


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


decodeGameMeta : D.Decoder GameMeta
decodeGameMeta =
    D.map4 GameMeta
        (D.field "turn" D.int)
        (D.field "phase" D.string)
        (D.field "activePlayer" D.string)
        (D.field "viewer" D.string)


{-| Pulls the slice the 11f/g/h render path needs out of the raw
`{state, prompt}` envelope stored in `Model.gameState`. Reads
`state.viewer` + `state.players[]`, splits into `you` / `opp`.
Fails if the player list isn't exactly two entries or neither side
matches the viewer — both indicate a malformed envelope.
-}
decodeGameViewSlice : D.Decoder GameViewSlice
decodeGameViewSlice =
    D.field "state"
        (D.map2 Tuple.pair
            (D.field "viewer" D.string)
            (D.field "players" (D.list decodePlayerCounts))
            |> D.andThen viewSliceFromPlayers
        )


viewSliceFromPlayers : ( String, List PlayerCounts ) -> D.Decoder GameViewSlice
viewSliceFromPlayers ( viewer, players ) =
    case players of
        [ a, b ] ->
            if a.side == viewer then
                D.succeed { viewer = viewer, you = a, opp = b }

            else if b.side == viewer then
                D.succeed { viewer = viewer, you = b, opp = a }

            else
                D.fail ("viewer " ++ viewer ++ " did not match any player side")

        _ ->
            D.fail "expected exactly two players"


decodePlayerCounts : D.Decoder PlayerCounts
decodePlayerCounts =
    D.succeed PlayerCounts
        |> required "side" D.string
        |> listOf "board" Card.decode
        |> listOf "hand" Card.decode
        |> listOf "graveyard" Card.decode
        |> required "deck_count" D.int
        |> required "hand_count" D.int
        |> required "exile_count" D.int
        |> required "graveyard_count" D.int
        |> optional "deck_top" Card.decode


{-| Pipeline-style decoder slot for a list field with an empty-list
fallback if the field is absent — used for opp.hand which the engine
filters out server-side rather than emitting `[]`.
-}
listOf : String -> D.Decoder a -> D.Decoder (List a -> b) -> D.Decoder b
listOf field aDec fDec =
    D.map2 (\f a -> f a)
        fDec
        (D.oneOf [ D.field field (D.list aDec), D.succeed [] ])


-- decodeDeckBack consolidated into Card.decode 2026-06-09. The
-- `optional "deck_top" Card.decode` above now produces a Maybe Card.Card
-- directly; null deck_top remains Nothing.


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
        , gameMetaIn GameMetaReceived
        , promptTextIn PromptTextReceived
        , gameStateIn GameStateReceived
        , spectatorStateIn SpectatorStateReceived
        , uctPreviewIn UctPreviewReceived
        ]



-- VIEW


view : Model -> Html Msg
view model =
    div []
        [ -- The card primitive owns its own CSS — mount once at top.
          -- Container layout (.cards/.pool-grid), contextual overrides
          -- (.opponent .card), and caller-specific decorations stay in
          -- play.html's <style>; card-internal rules live in Card.elm.
          Card.styles
        , -- Error primitive's CSS — overlay + LOG-mirror styling.
          -- Per ERROR.md the visual contract travels with the
          -- module, same as Card.styles.
          Error.styles
        , viewSaveControls model
        , viewSurfaceWithErrors "deckbuilder" model.errors (viewDeckbuilder model)
        , viewSurfaceWithErrors "spectator-bar" model.errors (SpectatorBar.view spectatorBarConfig model.spectatorBar)
        , viewSurfaceWithErrors "prompt" model.errors (viewPromptText model.promptText)
        , viewSurfaceWithErrors "game-meta" model.errors (viewGameMeta model.gameMeta)
        , viewSurfaceWithErrors "game-screen" model.errors (viewGameScreen model)
        , viewSavedListPanel model.savedList
        , viewDecisionPanel model.decisionPanel
        , LogPanel.view model.log
        , viewSurfaceWithErrors "build-footer" model.errors (BuildFooter.view model.build)
        ]


{-| Wrap a surface render in a `position: relative` container with
its surface-anchored errors stacked alongside. Per ERROR.md § Visual
contract this is the **fallback** anchoring (used by port-decode
failures with no cursor position); click-driven errors carrying a
cursor `Anchor` position themselves via `position: fixed` instead and
ignore the surface container.
-}
viewSurfaceWithErrors : String -> List Error.Error -> Html Msg -> Html Msg
viewSurfaceWithErrors surface errors child =
    div [ style "position" "relative" ]
        [ child
        , viewErrorsForSurface surface errors
        ]


spectatorBarConfig : SpectatorBar.Config Msg
spectatorBarConfig =
    { onBackEnd = SpecBackEndClicked
    , onStepBack = SpecStepBackClicked
    , onPlayPause = SpecPlayPauseClicked
    , onStepFwd = SpecStepFwdClicked
    , onFwdEnd = SpecFwdEndClicked
    , onSliderChange = SpecSliderChanged
    , onSpeedChange = SpecSpeedChanged
    , onExit = SpecExitClicked
    }


viewPromptText : String -> Html Msg
viewPromptText txt =
    div
        [ id "prompt"
        , style "padding" "0.5rem"
        , style "background" "#1a2030"
        , style "border" "1px solid #335"
        , style "margin-bottom" "0.75rem"
        ]
        [ text txt ]


viewGameMeta : Maybe GameMeta -> Html Msg
viewGameMeta maybeMeta =
    case maybeMeta of
        Nothing ->
            text ""

        Just m ->
            div
                [ class "meta"
                , style "color" "#888"
                , style "font-size" "0.75rem"
                , style "margin-bottom" "0.5rem"
                ]
                [ text
                    ("turn "
                        ++ String.fromInt m.turn
                        ++ " · phase "
                        ++ m.phase
                        ++ " · active "
                        ++ String.toUpper m.activePlayer
                        ++ " · you are "
                        ++ String.toUpper m.viewer
                    )
                ]


{-| Stage 11f/g/h — render the `#game-screen` zone scaffold + per-player
counts + deck-top backs. Card containers (`opp-board-cards`,
`opp-graveyard-cards`, `your-board-cards`, `your-graveyard-cards`,
`your-hand-cards`) are deliberately Elm-empty: play.html's `_renderInner`
still `appendChild`s into them by id. Elm's vdom diff never touches
their JS-injected children as long as vdom always sees `[]` here.

`#buttons` likewise stays JS-managed for now (Pass / Confirm / Cancel
+ prompt-kind branches arrive in 11m / 11n).

Hidden unless gamePhase is Playing or Spectating (so the deckbuilder
flow doesn't see an empty scaffold). Decode failure (malformed
envelope) returns nothing — the meta line + log will already have
surfaced the underlying problem.
-}
viewGameScreen : Model -> Html Msg
viewGameScreen model =
    let
        active =
            model.gamePhase == Playing || model.gamePhase == Spectating

        slice =
            case model.gameState of
                Just value ->
                    D.decodeValue decodeGameViewSlice value
                        |> Result.toMaybe

                Nothing ->
                    Nothing

        elmButtons =
            GameScreen.viewPromptButtons gameScreenButtonsConfig model.chooseIntDraft model.combat model.prompt
    in
    renderGameScreen active model.prompt model.combat slice elmButtons


gameScreenButtonsConfig : GameScreen.PromptButtonsConfig Msg
gameScreenButtonsConfig =
    { onConfirmYes = ConfirmYesClicked
    , onConfirmNo = ConfirmNoClicked
    , onPlayerChoice = PlayerChoiceClicked
    , onIntInput = IntChoiceInputChanged
    , onIntConfirm = IntChoiceConfirmClicked
    , onPass = PassClicked
    , onSkipChoiceCard = SkipChoiceCardClicked
    , onConfirmAttackers = ConfirmAttackersClicked
    , onNoAttack = NoAttackClicked
    , onConfirmBlocks = ConfirmBlocksClicked
    , onNoBlocks = NoBlocksClicked
    }


{-| Render the scaffold UNCONDITIONALLY — visibility toggles via inline
`display:none` when not Playing/Spectating. The card-container IDs
(`opp-board-cards`, etc.) must exist in the DOM from Elm's first
render, because `_renderInner` is called synchronously after
`setPhase(...)` in the load-save flow (and the start-game / spectate
paths only get away with it because they have an `await` between).
If `viewGameScreen` returned `text ""` for non-active phases, those
IDs would be absent at boot and the load-save sync `render()` would
hit "oppBoard is null" before Elm's first paint. Counts + deck-tops
fall back to placeholders when no `gameState` slice has landed.
-}
renderGameScreen : Bool -> GameScreen.Prompt -> GameScreen.CombatSelection -> Maybe GameViewSlice -> Html Msg -> Html Msg
renderGameScreen active prompt combat maybeSlice elmButtons =
    let
        oppCounts =
            Maybe.map (oppCountsText << .opp) maybeSlice |> Maybe.withDefault ""

        oppGy =
            Maybe.map (String.fromInt << .graveyardCount << .opp) maybeSlice |> Maybe.withDefault ""

        yourGy =
            Maybe.map (String.fromInt << .graveyardCount << .you) maybeSlice |> Maybe.withDefault ""

        yourHand =
            Maybe.map (yourHandCountsText << .you) maybeSlice |> Maybe.withDefault ""

        oppDeckTop =
            maybeSlice
                |> Maybe.andThen (.deckTop << .opp)
                |> Maybe.map (Card.view Card.faceDownConfig)
                |> Maybe.withDefault (text "")

        yourDeckTop =
            maybeSlice
                |> Maybe.andThen (.deckTop << .you)
                |> Maybe.map (Card.view Card.faceDownConfig)
                |> Maybe.withDefault (text "")

        displayStyle =
            if active then
                ""

            else
                "none"

        oppBoardCards =
            zoneCardsForPrompt OppBoard prompt combat maybeSlice (Maybe.map (.board << .opp) maybeSlice)

        oppGraveyardCards =
            zoneCardsForPrompt OppGraveyard prompt combat maybeSlice (Maybe.map (.graveyard << .opp) maybeSlice)

        yourBoardCards =
            zoneCardsForPrompt YourBoard prompt combat maybeSlice (Maybe.map (.board << .you) maybeSlice)

        yourGraveyardCards =
            zoneCardsForPrompt YourGraveyard prompt combat maybeSlice (Maybe.map (.graveyard << .you) maybeSlice)

        yourHandCards =
            zoneCardsForPrompt YourHand prompt combat maybeSlice (Maybe.map (.hand << .you) maybeSlice)
    in
    let
        oppDeckCount =
            Maybe.map (deckCountText << .opp) maybeSlice |> Maybe.withDefault ""

        yourDeckCount =
            Maybe.map (deckCountText << .you) maybeSlice |> Maybe.withDefault ""
    in
    -- Layout per user 2026-06-09: H1 D1 / B1 G1 / B2 G2 / H2 D2 —
    -- left column has hand→board→board→hand top-to-bottom, right column
    -- has deck→graveyard→graveyard→deck. Hand row's "hand" is the
    -- card list itself (your side) or just the count box (opp side
    -- since opp hand is hidden). Deck row's "deck" is the deck-top
    -- back-of-card widget plus a `deck:N` count badge.
    div [ id "game-screen", style "display" displayStyle ]
        [ div [ class "row" ]
            [ div [ class "zone opponent", style "flex" "2" ]
                [ h2 []
                    [ text "Opp hand "
                    , span [ class "counts", id "opp-counts" ] [ text oppCounts ]
                    ]
                , div [ class "cards", style "color" "#666", style "font-style" "italic", style "font-size" "0.7rem" ]
                    [ text "(hidden)" ]
                ]
            , div [ class "zone", style "flex" "0 0 14rem" ]
                [ h2 []
                    [ text "Opp deck "
                    , span [ class "counts" ] [ text oppDeckCount ]
                    ]
                , div [ class "cards", id "opp-deck-top" ] [ oppDeckTop ]
                ]
            ]
        , div [ class "row" ]
            [ div [ class "zone opponent", style "flex" "2" ]
                [ h2 [] [ text "Opp board" ]
                , Keyed.node "div" [ class "cards", id "opp-board-cards" ] oppBoardCards
                ]
            , div [ class "zone", style "flex" "0 0 14rem" ]
                [ h2 []
                    [ text "Opp graveyard "
                    , span [ class "counts", id "opp-gy-count" ] [ text oppGy ]
                    ]
                , Keyed.node "div" [ class "cards", id "opp-graveyard-cards" ] oppGraveyardCards
                ]
            ]
        , div [ class "row" ]
            [ div [ class "zone", style "flex" "2" ]
                [ h2 [] [ text "Your board" ]
                , Keyed.node "div" [ class "cards", id "your-board-cards" ] yourBoardCards
                ]
            , div [ class "zone", style "flex" "0 0 14rem" ]
                [ h2 []
                    [ text "Your graveyard "
                    , span [ class "counts", id "your-gy-count" ] [ text yourGy ]
                    ]
                , Keyed.node "div" [ class "cards", id "your-graveyard-cards" ] yourGraveyardCards
                ]
            ]
        , div [ class "row" ]
            [ div [ class "zone", style "flex" "2" ]
                [ h2 []
                    [ text "Your hand "
                    , span [ class "counts", id "your-hand-counts" ] [ text yourHand ]
                    ]
                , Keyed.node "div" [ class "cards", id "your-hand-cards" ] yourHandCards
                ]
            , div [ class "zone", style "flex" "0 0 14rem" ]
                [ h2 []
                    [ text "Your deck "
                    , span [ class "counts" ] [ text yourDeckCount ]
                    ]
                , div [ class "cards", id "your-deck-top" ] [ yourDeckTop ]
                ]
            ]
        , div [ id "buttons" ] []
        , elmButtons
        ]


-- Wave 5: defaultDimOpts + zoneCards retired — zoneCardsForPrompt is
-- the sole zone renderer, deriving dim/clickable/overlays from
-- (ZonePos × Prompt × CardView).


{-| Five game-screen card containers. Used by `zoneCardsForPrompt` to
dispatch per-prompt-kind `CardOpts` (e.g., graveyards default to
`dim = True`; ChooseCard's pool clickability applies to all 5; PickCard's
hand candidates only apply to YourHand; PickCard's board activations
only apply to YourBoard).
-}
type ZonePos
    = OppBoard
    | OppGraveyard
    | YourBoard
    | YourGraveyard
    | YourHand


{-| Per CARD.md Axiom Slice 2: returns keyed `(iid, html)` pairs so
the in-game zone containers can use `Html.Keyed.node` — intra-zone
reorderings (e.g. tap order, combat staging) preserve DOM identity.
The empty-state placeholder keeps a stable `"empty"` key so the
vDOM diffs it correctly when the zone goes from empty to populated.
-}
zoneCardsForPrompt : ZonePos -> GameScreen.Prompt -> GameScreen.CombatSelection -> Maybe GameViewSlice -> Maybe (List Card.Card) -> List ( String, Html Msg )
zoneCardsForPrompt zone prompt combat maybeSlice maybeCards =
    case maybeCards of
        Nothing ->
            []

        Just [] ->
            [ ( "empty", span [ class "empty-note" ] [ text "empty" ] ) ]

        Just cards ->
            let
                actsByIid =
                    case prompt of
                        GameScreen.PickCardPrompt data ->
                            List.foldl
                                (\a acc -> Dict.update a.iid (Maybe.withDefault [] >> (::) a >> Just) acc)
                                Dict.empty
                                data.activations

                        _ ->
                            Dict.empty
            in
            List.map
                (\c ->
                    ( Card.key c
                    , Card.view (cardOptsForZone zone prompt combat maybeSlice actsByIid c) c
                    )
                )
                cards


{-| Per-card opts: starts from the zone's baseline (graveyards dim;
others not), then layers any prompt-kind-specific decoration on top.
ChooseCard's overlay (pool clickability / host badge / non-pool dim)
takes precedence over the baseline because the engine guarantees the
prompt restricts the player to those pool members during this turn.
-}
cardOptsForZone : ZonePos -> GameScreen.Prompt -> GameScreen.CombatSelection -> Maybe GameViewSlice -> Dict String (List GameScreen.Activation) -> Card.Card -> Card.Config Msg
cardOptsForZone zone prompt combat maybeSlice actsByIid (Card.Card c) =
    let
        defaults =
            Card.defaultConfig

        baseDim =
            zone == OppGraveyard || zone == YourGraveyard

        base =
            { defaults | dim = baseDim }

        cIid =
            Maybe.withDefault "" c.iid
    in
    case prompt of
        GameScreen.ChooseCardPrompt data ->
            chooseCardOpts data (Card.Card c)

        GameScreen.PickCardPrompt data ->
            case zone of
                YourHand ->
                    if List.member cIid data.candidates then
                        { base | clickable = Just HandCardClicked }

                    else
                        base

                YourBoard ->
                    let
                        acts =
                            Dict.get cIid actsByIid
                                |> Maybe.withDefault []
                                |> List.reverse
                    in
                    case acts of
                        [] ->
                            base

                        first :: _ ->
                            { base
                                | clickable = Just (\_ -> BoardActivationClicked first)
                                , overlays = List.map activationRow acts
                            }

                _ ->
                    base

        GameScreen.PickAttackersPrompt data ->
            case zone of
                YourBoard ->
                    if List.member cIid data.eligible then
                        { base
                            | clickable = Just AttackerToggled
                            , selected = Set.member cIid combat.attackers
                        }

                    else
                        base

                _ ->
                    base

        GameScreen.PickBlocksPrompt data ->
            pickBlocksOpts data combat maybeSlice zone (Card.Card c) base

        _ ->
            base


{-| PickBlocks card opts. Three zones interact:

  - YourBoard: eligible blockers are clickable (stage / unstage /
    unassign), assigned blockers show the "→ blocks <attacker>"
    overlay, staged blocker shows "… click an attacker".
  - OppBoard: attacker iids get an orange border (#fa4). When a
    blocker is staged, all attackers become clickable targets;
    attackers with assigned blockers show "← blocked by …".

-}
pickBlocksOpts :
    GameScreen.PickBlocksData
    -> GameScreen.CombatSelection
    -> Maybe GameViewSlice
    -> ZonePos
    -> Card.Card
    -> Card.Config Msg
    -> Card.Config Msg
pickBlocksOpts data combat maybeSlice zone (Card.Card c) base =
    let
        cIid =
            Maybe.withDefault "" c.iid
    in
    case zone of
        YourBoard ->
            let
                isEligibleBlocker =
                    List.member cIid data.eligibleBlockers

                assignedTo =
                    Dict.get cIid combat.blocks

                isStaged =
                    combat.blockerPickFor == Just cIid
            in
            { base
                | clickable =
                    if isEligibleBlocker then
                        Just BlockerClicked

                    else
                        Nothing
                , selected = isStaged || assignedTo /= Nothing
                , overlays =
                    case assignedTo of
                        Just atkIid ->
                            [ blockerAssignmentLabel
                                ("\u{2192} blocks " ++ labelForIid maybeSlice atkIid)
                                "#6cf"
                            ]

                        Nothing ->
                            if isStaged then
                                [ blockerAssignmentLabel "\u{2026} click an attacker" "#fa4" ]

                            else
                                []
            }

        OppBoard ->
            let
                isAttacker =
                    List.member cIid data.attackers

                isClickableTarget =
                    isAttacker && combat.blockerPickFor /= Nothing

                blockersHere =
                    combat.blocks
                        |> Dict.toList
                        |> List.filterMap
                            (\( blkIid, atkIid ) ->
                                if atkIid == cIid then
                                    Just blkIid

                                else
                                    Nothing
                            )
            in
            { base
                | clickable =
                    if isClickableTarget then
                        Just AttackerTargetedForBlock

                    else
                        Nothing
                , borderColor =
                    if isAttacker then
                        Just "#fa4"

                    else
                        Nothing
                , overlays =
                    if List.isEmpty blockersHere then
                        []

                    else
                        [ blockerAssignmentLabel
                            ("\u{2190} blocked by "
                                ++ String.join ", " (List.map (labelForIid maybeSlice) blockersHere)
                            )
                            "#6cf"
                        ]
            }

        _ ->
            base


blockerAssignmentLabel : String -> String -> Html Msg
blockerAssignmentLabel txt color =
    div
        [ style "color" color
        , style "font-size" "0.6rem"
        , style "margin-top" "0.2rem"
        ]
        [ text txt ]


{-| Find a card's display name for the "→ blocks X" / "← blocked by X"
overlays. Searches every visible zone of both players for the iid;
falls back to the raw iid suffix when not found (shouldn't happen
mid-PickBlocks but stays safe).
-}
labelForIid : Maybe GameViewSlice -> String -> String
labelForIid maybeSlice iid =
    case maybeSlice of
        Nothing ->
            cardSuffixFromIid iid

        Just slice ->
            let
                allCards =
                    slice.you.board
                        ++ slice.you.hand
                        ++ slice.you.graveyard
                        ++ slice.opp.board
                        ++ slice.opp.graveyard
            in
            case List.filter (\(Card.Card c) -> c.iid == Just iid) allCards of
                (Card.Card c) :: _ ->
                    c.name

                [] ->
                    cardSuffixFromIid iid


chooseCardOpts : GameScreen.ChooseCardData -> Card.Card -> Card.Config Msg
chooseCardOpts data (Card.Card c) =
    let
        defaults =
            Card.defaultConfig

        cIid =
            Maybe.withDefault "" c.iid

        inPool =
            List.member cIid data.pool

        isHost =
            data.host == Just cIid
    in
    if isHost then
        { defaults
            | borderColor = Just "#fa4"
            , borderStyle = Just "dashed"
            , overlays = [ castingBadge ]
        }

    else if inPool then
        { defaults | clickable = Just TargetCardClicked }

    else
        { defaults | dim = True }


castingBadge : Html Msg
castingBadge =
    div
        [ style "color" "#fa4"
        , style "font-size" "0.6rem"
        , style "text-transform" "uppercase"
        , style "letter-spacing" "0.06em"
        , style "margin-top" "0.25rem"
        ]
        [ text "\u{25C6} casting" ]


activationRow : GameScreen.Activation -> Html Msg
activationRow a =
    let
        rawText =
            if String.length a.text > 60 then
                String.left 57 a.text ++ "\u{2026}"

            else
                a.text

        suffix =
            if a.needsX then
                " (X)"

            else
                ""
    in
    div
        [ style "color" "#6cf"
        , style "font-size" "0.65rem"
        , style "margin-top" "0.2rem"
        , style "padding" "0.15rem 0.3rem"
        , style "background" "#1a2030"
        , style "border" "1px solid #234"
        , style "cursor" "pointer"
        , Html.Events.stopPropagationOn "click"
            (D.succeed ( BoardActivationClicked a, True ))
        ]
        [ text ("\u{25B6} [" ++ String.fromInt a.abilityIndex ++ "] " ++ rawText ++ suffix) ]


{-| Opponent's hand-count + exile-count (deck-count moved to the
deck-top widget per user layout 2026-06-09). -}
oppCountsText : PlayerCounts -> String
oppCountsText p =
    "hand:" ++ String.fromInt p.handCount ++ " ex:" ++ String.fromInt p.exileCount


yourHandCountsText : PlayerCounts -> String
yourHandCountsText p =
    "ex:" ++ String.fromInt p.exileCount


{-| Deck-zone header text: shows the deck size. Lives next to the
deck-top widget now (used to be in the hand-counts zone). -}
deckCountText : PlayerCounts -> String
deckCountText p =
    "deck:" ++ String.fromInt p.deckCount


-- viewDeckTop / colorTagEl / symbolTagEl removed 2026-06-09. Deck-top
-- now renders via `Card.view Card.defaultConfig Card.Back` (see
-- renderGameScreen). The old impl also painted color tags on the back
-- — that's gone per RULES C.1 (back shows symbols only).


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
    Keyed.node "div"
        [ class "pool-grid"
        , style "display" "flex"
        , style "flex-wrap" "wrap"
        , style "gap" "0.3rem"
        , style "max-height" "calc(100vh - 16rem)"
        , style "overflow-y" "auto"
        ]
        (List.map (\e -> ( e.id, viewPoolCard e )) visible)


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
    Card.view (poolCardConfig entry) (poolEntryToCard entry)


poolCardConfig : CardPoolEntry -> Card.Config Msg
poolCardConfig _ =
    let
        base =
            Card.defaultConfig
    in
    { base | clickable = Just PoolCardClicked }


{-| Bridge a deckbuilder pool entry into the unified `Card`. Phase 3
of card consolidation — the deckbuilder previously had its own
ad-hoc render (`viewPoolCard` + `viewCardMetaLine` + `colorTag` etc.)
that drifted from the in-game render. Now both surfaces route through
`Card.view Card.Front`.

Pool entries lack the in-game-only fields (`iid`, `tapped`,
`summoning_sick`, `damage`, `effective_cost`, `attached`); they default
to the values a fresh-printed card would have. Power/toughness default
to 0 when the kind is non-Creature — `Card.viewMeta` only emits the
stats line when `kind == Creature`, so the 0/0 default never renders
for non-creatures.

Symbols default to the SLOTS.md spiral-out order (same fallback as
`Card.decode`). When the engine emits per-slot positions on the pool
wire shape, only this converter changes.
-}
poolEntryToCard : CardPoolEntry -> Card.Card
poolEntryToCard e =
    Card.Card
        { iid = Nothing
        , id = e.id
        , name = e.name
        , kind = Card.kindFromString e.kind
        , colors = e.colors
        , symbols = List.map2 Card.SlotSymbol Card.slotSpiralOrder e.symbols
        , subtypes = e.subtypes
        , printedCost = e.costText
        , effectiveCost = e.costText
        , abilities = e.abilities
        , timing = Maybe.andThen parsePoolTiming e.timing
        , transparentFrame = False
        , holes = []
        , printedPower = Maybe.withDefault 0 e.power
        , printedToughness = Maybe.withDefault 0 e.toughness
        , tapped = False
        , summoningSick = False
        , damage = 0
        , attached = []
        }


parsePoolTiming : String -> Maybe Card.Timing
parsePoolTiming s =
    case String.toLower s of
        "instant" ->
            Just Card.Instant

        "sorcery" ->
            Just Card.Sorcery

        _ ->
            Nothing


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


{-| Render the current deck as a vertical column of full `Card.Front`
renders, each with qty + remove overlay. Per user 2026-06-09 — "always
need to see the full card", "a compact list is harmful because it
hides information" — so the deck list shows the same primitive as the
pool, not a compact pill row. Ids without a matching pool entry are
dropped (stale deck on a freshly-loaded pool).
-}
viewDeckRows : Model -> Html Msg
viewDeckRows model =
    let
        counts =
            countById model.deck

        entryFor id =
            model.cardPool
                |> List.filter (\e -> e.id == id)
                |> List.head

        nameOf id =
            entryFor id |> Maybe.map .name |> Maybe.withDefault id

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
    Keyed.node "div"
        [ style "display" "flex"
        , style "flex-direction" "column"
        , style "gap" "0.3rem"
        ]
        (List.filterMap
            (\( id, qty ) ->
                entryFor id |> Maybe.map (\entry -> ( entry.id, viewDeckRow qty entry ))
            )
            sorted
        )


viewDeckRow : Int -> CardPoolEntry -> Html Msg
viewDeckRow qty entry =
    Card.view (deckRowConfig qty entry) (poolEntryToCard entry)


deckRowConfig : Int -> CardPoolEntry -> Card.Config Msg
deckRowConfig qty entry =
    let
        base =
            Card.defaultConfig
    in
    { base
        | overlays =
            [ div
                [ class "deck-row-controls"
                , style "display" "flex"
                , style "align-items" "center"
                , style "justify-content" "space-between"
                , style "gap" "0.4rem"
                , style "margin-top" "0.3rem"
                ]
                [ span
                    [ class "qty"
                    , style "color" "#fc6"
                    , style "font-size" "0.75rem"
                    ]
                    [ text (String.fromInt qty ++ "\u{00D7}") ]
                , button
                    [ onClick (DeckRowRemove entry.id)
                    , style "padding" "0.1rem 0.4rem"
                    , style "font-size" "0.7rem"
                    ]
                    [ text "-" ]
                ]
            ]
    }


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


-- viewLog + viewBuildFooter + their helpers moved to LogPanel / BuildFooter.


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
