port module Roam.Main exposing (main)

import Browser
import Browser.Events as BE
import Dict exposing (Dict)
import Error exposing (Error)
import Html exposing (Html, canvas, div, h2, header, section, span, text)
import Html.Attributes as A
import Html.Keyed as Keyed
import Json.Decode as D


-- MODEL --------------------------------------------------------------


type alias Flags =
    { viewport : { w : Float, h : Float }
    , buildLabel : Maybe String
    }


type alias DragState =
    { errorId : String
    , offsetX : Float
    , offsetY : Float
    }


type alias Model =
    { errors : List Error
    , dragging : Maybe DragState
    , positions : Dict String Error.Anchor
    , viewport : { w : Float, h : Float }
    , buildLabel : Maybe String
    }


init : Flags -> ( Model, Cmd Msg )
init flags =
    ( { errors = []
      , dragging = Nothing
      , positions = Dict.empty
      , viewport = flags.viewport
      , buildLabel = flags.buildLabel
      }
    , Cmd.none
    )



-- UPDATE -------------------------------------------------------------


type Msg
    = ErrorIn D.Value
    | DismissError String
    | DragStarted String Error.DragOffset
    | MouseMoved Error.Anchor
    | MouseReleased
    | ViewportResize Int Int


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        ErrorIn raw ->
            case D.decodeValue Error.decode raw of
                Ok e ->
                    ( { model | errors = e :: model.errors }, Cmd.none )

                Err _ ->
                    -- TODO: surface envelope decode failure as a typed
                    -- Error so the axiom doesn't silently drop it.
                    ( model, Cmd.none )

        DismissError id ->
            if id == "" then
                ( model, Cmd.none )

            else
                ( { model
                    | errors = List.filter (\e -> Error.key e /= id) model.errors
                    , positions = Dict.remove id model.positions
                  }
                , Cmd.none
                )

        DragStarted id offset ->
            ( { model | dragging = Just { errorId = id, offsetX = offset.offsetX, offsetY = offset.offsetY } }
            , Cmd.none
            )

        MouseMoved cursor ->
            case model.dragging of
                Just drag ->
                    let
                        pos =
                            { x = cursor.x - drag.offsetX
                            , y = cursor.y - drag.offsetY
                            }
                    in
                    ( { model | positions = Dict.insert drag.errorId pos model.positions }
                    , Cmd.none
                    )

                Nothing ->
                    ( model, Cmd.none )

        MouseReleased ->
            ( { model | dragging = Nothing }, Cmd.none )

        ViewportResize w h ->
            ( { model | viewport = { w = toFloat w, h = toFloat h } }, Cmd.none )



-- VIEW ---------------------------------------------------------------


view : Model -> Html Msg
view model =
    div []
        [ Error.styles
        , header [ A.id "build" ]
            (case model.buildLabel of
                Just label ->
                    [ text label ]

                Nothing ->
                    []
            )
        , section [ A.id "wrap" ]
            [ div [ A.id "game" ]
                [ canvas [ A.id "c" ] []
                , div [ A.id "status" ] []
                ]
            , div [ A.id "info" ]
                [ panel "inventory" [ div [ A.id "inv" ] [] ]
                , panel "self" [ div [ A.id "self" ] [] ]
                , panel "libp2p connections" [ div [ A.id "conns" ] [] ]
                , panel "gossipsub mesh — topic peers" [ div [ A.id "mesh" ] [] ]
                , panel "event log" [ Html.pre [ A.id "log" ] [] ]
                ]
            ]
        , Keyed.node "div"
            [ A.id "roam-errors" ]
            (List.map (\e -> ( Error.key e, viewError model e )) model.errors)
        ]


viewError : Model -> Error -> Html Msg
viewError model e =
    Error.view
        { onDismiss = DismissError
        , onDragStart = DragStarted
        , position = Dict.get (Error.key e) model.positions
        , viewport = model.viewport
        , buildLabel = model.buildLabel
        }
        e


panel : String -> List (Html Msg) -> Html Msg
panel title body =
    div [ A.class "panel" ]
        (h2 [] [ text title ] :: body)



-- PORTS --------------------------------------------------------------


port errorIn : (D.Value -> msg) -> Sub msg



-- SUBSCRIPTIONS ------------------------------------------------------


subscriptions : Model -> Sub Msg
subscriptions model =
    Sub.batch
        [ errorIn ErrorIn
        , BE.onResize ViewportResize
        , case model.dragging of
            Just _ ->
                Sub.batch
                    [ BE.onMouseMove
                        (D.map MouseMoved
                            (D.map2 Error.Anchor
                                (D.field "clientX" D.float)
                                (D.field "clientY" D.float)
                            )
                        )
                    , BE.onMouseUp (D.succeed MouseReleased)
                    ]

            Nothing ->
                Sub.none
        ]



-- MAIN ---------------------------------------------------------------


main : Program Flags Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        }
