/* hello.c: Erk from C. A windowless app opens a page, finds its button,
 * subscribes to clicks on it and clicks it; the callback changes the page.
 * CI builds and runs it on every platform, under AddressSanitizer on Linux
 * (p1-contract §11): it also checks that Erk copies the strings it is given,
 * fills buffers whole or not at all, and calls each destroy exactly once. */

#include <stdio.h>
#include <string.h>

#include "erk.h"

static int failures = 0;

#define CHECK(condition)                                                     \
  do {                                                                       \
    if (!(condition)) {                                                      \
      fprintf(stderr, "%s:%d: %s\n", __FILE__, __LINE__, #condition);       \
      failures++;                                                            \
    }                                                                        \
  } while (0)

static ErkStr str(const char *text) {
  ErkStr s = {text, strlen(text)};
  return s;
}

struct Counter {
  int clicks;
  int destroyed;
  ErkNodeId label;
};

static void on_click(void *user_data, ErkApp *app, const ErkEvent *event) {
  struct Counter *counter = user_data;
  CHECK(event->kind == ERK_EVENT_CLICK);
  counter->clicks++;
  /* Inside a callback the document may change; the loop may not run. */
  CHECK(erk_node_set_text(app, counter->label, str("tıklandı")) == ERK_OK);
  CHECK(erk_app_tick(app, 0) == ERK_ERR_REENTRANT);
}

static void on_destroy(void *user_data) {
  struct Counter *counter = user_data;
  counter->destroyed++;
}

int main(void) {
  CHECK(erk_abi_version() == ((0u << 16) | 5u));

  ErkConfig config;
  memset(&config, 0, sizeof config);
  config.struct_size = sizeof config;
  config.width = 320;
  config.height = 200;
  config.flags = ERK_APP_HEADLESS | ERK_APP_EMBEDDED_FONTS;
  ErkApp *app = NULL;
  CHECK(erk_app_create(&config, &app) == ERK_OK);
  if (app == NULL) {
    return 1;
  }

  /* Erk copies the page before the call returns: overwriting it after
   * changes nothing. */
  char page[] = "<body style=\"margin: 0\"><button id=\"b\" style=\"display: block; "
                "width: 100px; height: 40px\">Tıkla</button><p id=\"label\">bekliyor</p>";
  CHECK(erk_load_html(app, str(page)) == ERK_OK);
  memset(page, 'x', sizeof page - 1);
  CHECK(erk_app_tick(app, 1) == ERK_OK);

  ErkNodeId button = ERK_NODE_NONE, label = ERK_NODE_NONE;
  CHECK(erk_query(app, ERK_NODE_NONE, str("#b"), &button) == ERK_OK);
  CHECK(erk_query(app, ERK_NODE_NONE, str("#label"), &label) == ERK_OK);
  CHECK(button != ERK_NODE_NONE && label != ERK_NODE_NONE);

  /* The host builds part of the page: a new paragraph after the label. */
  ErkNodeId body = ERK_NODE_NONE, note = ERK_NODE_NONE, note_text = ERK_NODE_NONE;
  CHECK(erk_query(app, ERK_NODE_NONE, str("body"), &body) == ERK_OK);
  CHECK(erk_node_create(app, str("p"), &note) == ERK_OK);
  CHECK(erk_text_create(app, str("C'den eklendi"), &note_text) == ERK_OK);
  CHECK(erk_node_append(app, note, note_text) == ERK_OK);
  CHECK(erk_node_set_attr(app, note, str("id"), str("note")) == ERK_OK);
  CHECK(erk_node_append(app, body, note) == ERK_OK);
  ErkNodeId found = ERK_NODE_NONE;
  CHECK(erk_query(app, ERK_NODE_NONE, str("body > p#note"), &found) == ERK_OK);
  CHECK(found == note);
  /* Into itself: refused. */
  CHECK(erk_node_append(app, note, note) == ERK_ERR_INVALID_ARGUMENT);

  /* The same in one call: a list of two items, the second naming the
   * first's element by its position in the batch. */
  ErkMutation batch[5];
  memset(batch, 0, sizeof batch);
  for (size_t i = 0; i < 5; i++) {
    batch[i].struct_size = sizeof batch[i];
  }
  batch[0].kind = ERK_MUTATION_CREATE_ELEMENT;
  batch[0].value = str("ul");
  batch[1].kind = ERK_MUTATION_CREATE_ELEMENT;
  batch[1].value = str("li");
  batch[2].kind = ERK_MUTATION_SET_TEXT;
  batch[2].node = ERK_NEW_NODE + 1;
  batch[2].value = str("toplu");
  batch[3].kind = ERK_MUTATION_APPEND;
  batch[3].parent = ERK_NEW_NODE + 0;
  batch[3].node = ERK_NEW_NODE + 1;
  batch[4].kind = ERK_MUTATION_APPEND;
  batch[4].parent = body;
  batch[4].node = ERK_NEW_NODE + 0;
  ErkNodeId created[5];
  size_t failed_at = 0;
  CHECK(erk_apply(app, batch, 5, created, &failed_at) == ERK_OK);
  ErkNodeId items[2] = {ERK_NODE_NONE, ERK_NODE_NONE};
  size_t count = 0;
  CHECK(erk_query_all(app, ERK_NODE_NONE, str("ul > li"), items, 2, &count) == ERK_OK);
  CHECK(count == 1 && items[0] == created[1] && created[3] == ERK_NODE_NONE);

  /* A buffer too small is not written; *len says what is needed. */
  char small[4] = {'-', '-', '-', '-'};
  size_t len = 0;
  CHECK(erk_node_text(app, label, small, sizeof small, &len) == ERK_ERR_BUFFER_TOO_SMALL);
  CHECK(len == strlen("bekliyor"));
  CHECK(memcmp(small, "----", 4) == 0);

  struct Counter counter = {0, 0, label};
  ErkSubscription subscription = 0;
  CHECK(erk_on(app, button, ERK_EVENT_CLICK, on_click, &counter, on_destroy, &subscription) ==
        ERK_OK);

  /* Click the middle of the button's box. */
  ErkBox box;
  memset(&box, 0, sizeof box);
  box.struct_size = sizeof box;
  CHECK(erk_node_box(app, button, &box) == ERK_OK);
  ErkInput input;
  memset(&input, 0, sizeof input);
  input.struct_size = sizeof input;
  input.x = box.x + box.width / 2;
  input.y = box.y + box.height / 2;
  input.button = ERK_BUTTON_PRIMARY;
  input.kind = ERK_INPUT_POINTER_DOWN;
  CHECK(erk_app_input(app, &input) == ERK_OK);
  input.kind = ERK_INPUT_POINTER_UP;
  CHECK(erk_app_input(app, &input) == ERK_OK);
  CHECK(counter.clicks == 1);

  char text[64];
  CHECK(erk_node_text(app, label, text, sizeof text, &len) == ERK_OK);
  CHECK(len == strlen("tıklandı") && memcmp(text, "tıklandı", len) == 0);
  CHECK(erk_app_tick(app, 2) == ERK_OK);
  ErkFrame frame;
  memset(&frame, 0, sizeof frame);
  frame.struct_size = sizeof frame;
  CHECK(erk_app_frame(app, &frame) == ERK_OK);
  CHECK(frame.width == 320 && frame.height == 200 && frame.len == 320u * 200u * 4u);

  /* A new document: the old ids are stale, and the subscription ended with
   * its node, its destroy called once. */
  CHECK(erk_load_html(app, str("<p>yeni</p>")) == ERK_OK);
  CHECK(erk_node_text(app, label, text, sizeof text, &len) == ERK_ERR_STALE_NODE);
  CHECK(counter.destroyed == 1);
  CHECK(erk_off(app, subscription) == ERK_ERR_NOT_FOUND);

  ErkString style = {NULL, 0};
  ErkNodeId root = ERK_NODE_NONE;
  CHECK(erk_document_root(app, &root) == ERK_OK);
  CHECK(erk_node_computed_style(app, root, &style) == ERK_ERR_NOT_FOUND);
  erk_string_free(style);

  CHECK(erk_app_destroy(app) == ERK_OK);
  CHECK(counter.destroyed == 1);

  if (failures != 0) {
    fprintf(stderr, "%d checks failed\n", failures);
    return 1;
  }
  printf("ok\n");
  return 0;
}
