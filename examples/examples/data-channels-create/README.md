# data-channels-create

data-channels-create is a WebRTC.rs application that shows how you can send/recv DataChannel messages from a web browser. The difference with the data-channels example is that the datachannel is initialized from the WebRTC.rs side in this example.

## Instructions

### Build data-channels-create

```shell
cargo build --example data-channels-create
```

### Open data-channels-create example page

[jsfiddle.net](https://jsfiddle.net/swgxrp94/20/)

### Run data-channels-create

Just run `data-channels-create`.

### Input data-channels-create's SessionDescription into your browser

Copy the text that `data-channels-create` just emitted and copy into first text area of the jsfiddle.

### Hit 'Start Session' in jsfiddle

Hit the 'Start Session' button in the browser. You should see `have-remote-offer` below the `Send Message` button.

### Input browser's SessionDescription into data-channels-create

Meanwhile text has appeared in the second text area of the jsfiddle. Copy the text and paste it into `data-channels-create` and hit ENTER.
In the browser you'll now see `connected` as the connection is created. If everything worked you should see `New DataChannel data`.

Now you can put whatever you want in the `Message` textarea, and when you hit `Send Message` it should appear in your terminal!

WebRTC.rs will send random messages every 5 seconds that will appear in your browser.

Congrats, you have used WebRTC.rs!
