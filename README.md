Command-line Pandora client.

# Features
* User-editable JSON configuration file
* Default configuration file can be generated with a command line flag
* Visually select a station from a list
* Display current track, playback time, and volume
* Rating tracks (thumbs-up/down), and removing the rating from a track
* Support for caching tracks before playing them, providing robustness against network issues during playback
* Keybindings:

  | Key | Action |
  | --- | ------ |
  | q | Quit |
  | . | Pause |
  | > | Unpause |
  | p | Toggle pause |
  | ( | Volume down |
  | ) | Volume up |
  | n | Skip to next track |
  | t | Track is 'tired', suspend it for a month |
  | + | Thumbs-up track |
  | - | Thumbs-down track |
  | = | Clear track rating |

# TODO
* Add menubar with entries corresponding to most hotkeys
* Add keybinding configuration
* Add user-configurable themes
* Robustness against Pandora session errors, e.g.

  ```
  [panharmonicon::model] src/model.rs:***: Failed while fetching new playlist: Pandora connection error: Pandora API error: Pandora API Call Error (Insufficient Connectivity Error): An unexpected error occurred
  ```
