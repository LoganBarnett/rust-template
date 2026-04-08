module Main exposing (main)

import Browser
import Browser.Navigation as Nav
import Html exposing (..)
import Html.Attributes exposing (..)
import Http
import Json.Decode as Decode
import Url exposing (Url)
import Url.Parser exposing (Parser, oneOf, top)


type Route
    = Home
    | Me
    | NotFound


routeParser : Parser (Route -> a) a
routeParser =
    oneOf
        [ Url.Parser.map Home top
        , Url.Parser.map Me (Url.Parser.s "me")
        ]


routeFromUrl : Url -> Route
routeFromUrl url =
    Url.Parser.parse routeParser url
        |> Maybe.withDefault NotFound


type alias MeInfo =
    { name : String
    , authEnabled : Bool
    }


type MeStatus
    = Loading
    | Loaded MeInfo
    | Failed


type alias Model =
    { key : Nav.Key
    , url : Url
    , route : Route
    , me : MeStatus
    }


type Msg
    = UrlRequested Browser.UrlRequest
    | UrlChanged Url
    | GotMe (Result Http.Error MeInfo)


main : Program () Model Msg
main =
    Browser.application
        { init = init
        , view = view
        , update = update
        , subscriptions = \_ -> Sub.none
        , onUrlRequest = UrlRequested
        , onUrlChange = UrlChanged
        }


init : () -> Url -> Nav.Key -> ( Model, Cmd Msg )
init _ url key =
    let
        route =
            routeFromUrl url
    in
    ( { key = key, url = url, route = route, me = Loading }
    , cmdForRoute route
    )


cmdForRoute : Route -> Cmd Msg
cmdForRoute route =
    case route of
        Me ->
            fetchMe

        _ ->
            Cmd.none


fetchMe : Cmd Msg
fetchMe =
    Http.get
        { url = "/me"
        , expect = Http.expectJson GotMe meDecoder
        }


meDecoder : Decode.Decoder MeInfo
meDecoder =
    Decode.map2 MeInfo
        (Decode.field "name" Decode.string)
        (Decode.field "auth_enabled" Decode.bool)


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        UrlRequested (Browser.Internal url) ->
            ( model, Nav.pushUrl model.key (Url.toString url) )

        UrlRequested (Browser.External url) ->
            ( model, Nav.load url )

        UrlChanged url ->
            let
                route =
                    routeFromUrl url
            in
            ( { model | url = url, route = route, me = Loading }
            , cmdForRoute route
            )

        GotMe result ->
            case result of
                Ok info ->
                    ( { model | me = Loaded info }, Cmd.none )

                Err _ ->
                    ( { model | me = Failed }, Cmd.none )


view : Model -> Browser.Document Msg
view model =
    { title = "rust-template"
    , body =
        [ main_ []
            [ nav []
                [ a [ href "/" ] [ text "Home" ]
                , text " | "
                , a [ href "/me" ] [ text "Me" ]
                , text " | "
                , a [ href "/scalar" ] [ text "API docs" ]
                ]
            , viewPage model
            ]
        ]
    }


viewPage : Model -> Html Msg
viewPage model =
    case model.route of
        Home ->
            div []
                [ h1 [] [ text "rust-template" ]
                , p [] [ text "Your application is running." ]
                ]

        Me ->
            viewMe model.me

        NotFound ->
            div []
                [ h1 [] [ text "Not found" ]
                , p [] [ text "The page you requested does not exist." ]
                ]


viewMe : MeStatus -> Html Msg
viewMe status =
    case status of
        Loading ->
            p [] [ text "Loading..." ]

        Failed ->
            p [] [ text "Failed to load user information." ]

        Loaded info ->
            div []
                [ h1 [] [ text "Me" ]
                , p [] [ text ("Name: " ++ info.name) ]
                , p []
                    [ text
                        ("Authentication: "
                            ++ (if info.authEnabled then
                                    "enabled"

                                else
                                    "disabled"
                               )
                        )
                    ]
                ]
