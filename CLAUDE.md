## Project

This is zellij-tools, a zellij plugin that adds a few handy utilities to zellij. It primarily operates by listening for messages over the pipe. From the Zellij docs:

> ## The pipe lifecycle method
>
> Plugins may listen to pipes by implementing the pipe lifecycle method. This method is called every time a message is sent over a pipe to this plugin (whether it's broadcast to all plugins or specifically directed at this one). It receives a PipeMessage containing the source of the pipe (CLI, another plugin or a keybinding), as well as information about said source (the plugin id or the CLI pipe id). The PipeMessage also contains the name of the pipe (explicitly provided by the user or a random UUID assigned by Zellij), its payload if it has one, its arguments and whether it is private or not (a private message is one directed specifically at this plugin rather than broadcast to all plugins).
>
> Similar to the update method, the pipe lifecycle method returns a bool, true if it would like to render itself, in which case the render function will be called as normal.
>
> Here's a small Rust example:
>
> ```rust
> fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
>     let mut should_render = false;
>     match pipe_message.source {
>         PipeSource::Cli(input_pipe_id) => {
>             if let Some(payload) = pipe_message.payload {
>                 self.messages_from_cli.push(payload);
>                 should_render = true;
>             }
>             if self.paused {
>                 // backpressure, this will pause data from the CLI pipeline until the unblock_cli_pipe_input method will be called for this id
>                 // from this or another plugin
>                 block_cli_pipe_input(&input_pipe_id);
>             }
>             if self.should_print_to_cli_stdout {
>                 // this can happen anywhere, anytime, from multiple plugins and is not tied to data from STDIN
>                 // as long as the pipe is open, plugins with its ID can print arbitrary data to its STDOUT side, even if the input side is blocked
>                 cli_pipe_output(input_pipe_id, &payload);
>             }
>         }
>         PipeSource::Plugin(source_plugin_id) => {
>             // pipes can also arrive from other plugins
>         }
>     }
>     should_render
> }
> ```

## Building

- To build, use `mise build-release`

