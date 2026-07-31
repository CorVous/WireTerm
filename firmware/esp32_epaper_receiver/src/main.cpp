/*
 * WireTerm offline ESP32 display bridge firmware.
 * Accepts one complete CRC32-verified 800x480 B/W/R frame before refreshing.
 */

#include <Arduino.h>
#include <SPI.h>
#include <WiFi.h>
#include <esp_bt.h>

namespace Pins {
constexpr uint8_t DIN = 14;
constexpr uint8_t CLK = 13;
constexpr uint8_t CS = 15;
constexpr uint8_t DC = 27;
constexpr uint8_t RST = 26;
constexpr uint8_t BUSY = 25;
constexpr uint8_t PWR = 32;
}  // namespace Pins

constexpr uint16_t FRAME_WIDTH = 800;
constexpr uint16_t FRAME_HEIGHT = 480;
constexpr size_t LINE_CAPACITY = 128;
constexpr size_t PLANE_BYTES = FRAME_WIDTH * FRAME_HEIGHT / 8U;
constexpr size_t FRAME_BYTES = PLANE_BYTES * 2U;
constexpr uint32_t BUSY_TIMEOUT_MS = 45000;
constexpr uint32_t RECEIVE_IDLE_TIMEOUT_MS = 3000;

char line[LINE_CAPACITY] = {};
size_t lineLength = 0;
uint8_t* frameBuffer = nullptr;
bool receivingFrame = false;
size_t receivedBytes = 0;
uint32_t expectedCrc = 0;
uint32_t runningCrc = 0xFFFFFFFFU;
uint32_t lastByteAt = 0;

void panelSafeOff() {
  pinMode(Pins::PWR, OUTPUT);
  digitalWrite(Pins::PWR, LOW);
  pinMode(Pins::CS, OUTPUT);
  digitalWrite(Pins::CS, HIGH);
  pinMode(Pins::DC, OUTPUT);
  digitalWrite(Pins::DC, LOW);
  pinMode(Pins::RST, OUTPUT);
  digitalWrite(Pins::RST, LOW);
  pinMode(Pins::DIN, INPUT);
  pinMode(Pins::CLK, INPUT);
  pinMode(Pins::BUSY, INPUT);
}

void sendCommand(uint8_t command) {
  digitalWrite(Pins::DC, LOW);
  digitalWrite(Pins::CS, LOW);
  SPI.transfer(command);
  digitalWrite(Pins::CS, HIGH);
}

void sendData(uint8_t data) {
  digitalWrite(Pins::DC, HIGH);
  digitalWrite(Pins::CS, LOW);
  SPI.transfer(data);
  digitalWrite(Pins::CS, HIGH);
}

bool waitUntilIdle() {
  const uint32_t started = millis();
  while (digitalRead(Pins::BUSY) == LOW) {
    sendCommand(0x71);
    if (millis() - started >= BUSY_TIMEOUT_MS) {
      return false;
    }
    delay(10);
  }
  delay(200);
  return true;
}

bool initializePanel() {
  pinMode(Pins::CS, OUTPUT);
  pinMode(Pins::RST, OUTPUT);
  pinMode(Pins::DC, OUTPUT);
  pinMode(Pins::BUSY, INPUT);
  pinMode(Pins::PWR, OUTPUT);
  digitalWrite(Pins::CS, HIGH);
  digitalWrite(Pins::PWR, HIGH);

  SPI.begin(Pins::CLK, -1, Pins::DIN, Pins::CS);
  SPI.beginTransaction(SPISettings(2000000, MSBFIRST, SPI_MODE0));

  digitalWrite(Pins::RST, HIGH);
  delay(200);
  digitalWrite(Pins::RST, LOW);
  delay(2);
  digitalWrite(Pins::RST, HIGH);
  delay(200);

  sendCommand(0x01);
  sendData(0x07);
  sendData(0x07);
  sendData(0x3F);
  sendData(0x3F);
  sendCommand(0x04);
  delay(100);
  if (!waitUntilIdle()) {
    return false;
  }

  sendCommand(0x00);
  sendData(0x0F);
  sendCommand(0x61);
  sendData(0x03);
  sendData(0x20);
  sendData(0x01);
  sendData(0xE0);
  sendCommand(0x15);
  sendData(0x00);
  sendCommand(0x50);
  sendData(0x11);
  sendData(0x07);
  sendCommand(0x60);
  sendData(0x22);
  sendCommand(0x65);
  sendData(0x00);
  sendData(0x00);
  sendData(0x00);
  sendData(0x00);
  return true;
}

void endPanelSession(bool enterSleep) {
  if (enterSleep) {
    sendCommand(0x02);
    if (waitUntilIdle()) {
      sendCommand(0x07);
      sendData(0xA5);
    }
  }
  SPI.endTransaction();
  SPI.end();
  panelSafeOff();
}

bool displayPlanes(const uint8_t* blackPlane, const uint8_t* redPlane) {
  if (!initializePanel()) {
    panelSafeOff();
    return false;
  }

  sendCommand(0x10);
  for (size_t index = 0; index < PLANE_BYTES; ++index) {
    sendData(blackPlane[index]);
    if (index % 1000U == 0) {
      yield();
    }
  }

  sendCommand(0x13);
  for (size_t index = 0; index < PLANE_BYTES; ++index) {
    sendData(redPlane[index]);
    if (index % 1000U == 0) {
      yield();
    }
  }

  sendCommand(0x12);
  delay(100);
  const bool refreshed = waitUntilIdle();
  endPanelSession(refreshed);
  return refreshed;
}

bool displayPolarityTest() {
  memset(frameBuffer, 0xFF, PLANE_BYTES);
  memset(frameBuffer + PLANE_BYTES, 0x00, PLANE_BYTES);
  for (uint16_t y = 0; y < FRAME_HEIGHT; ++y) {
    const size_t row = static_cast<size_t>(y) * (FRAME_WIDTH / 8U);
    memset(frameBuffer + row, 0x00, 33);
    memset(frameBuffer + PLANE_BYTES + row + 33, 0xFF, 33);
  }
  return displayPlanes(frameBuffer, frameBuffer + PLANE_BYTES);
}

uint32_t updateCrc32(uint32_t crc, uint8_t value) {
  crc ^= value;
  for (uint8_t bit = 0; bit < 8; ++bit) {
    const uint32_t mask = 0U - (crc & 1U);
    crc = (crc >> 1U) ^ (0xEDB88320U & mask);
  }
  return crc;
}

void resetReceiver() {
  receivingFrame = false;
  receivedBytes = 0;
  expectedCrc = 0;
  runningCrc = 0xFFFFFFFFU;
}

void finishFrame() {
  const uint32_t actualCrc = ~runningCrc;
  receivingFrame = false;
  if (actualCrc != expectedCrc) {
    Serial.printf("ERR CRC expected=%08lX actual=%08lX\n",
                  static_cast<unsigned long>(expectedCrc),
                  static_cast<unsigned long>(actualCrc));
    resetReceiver();
    return;
  }

  Serial.printf("OK FRAME VERIFIED bytes=%u crc=%08lX\n",
                static_cast<unsigned>(FRAME_BYTES),
                static_cast<unsigned long>(actualCrc));
  if (displayPlanes(frameBuffer, frameBuffer + PLANE_BYTES)) {
    Serial.println("OK FRAME DISPLAYED panel_power=OFF");
  } else {
    Serial.println("ERR DISPLAY_TIMEOUT panel_power=OFF");
  }
  resetReceiver();
}

bool beginFrame(const char* command) {
  unsigned width = 0;
  unsigned height = 0;
  unsigned length = 0;
  char format[8] = {};
  char crcHex[9] = {};
  if (sscanf(command, "BEGIN %u %u %7s %u %8s",
             &width, &height, format, &length, crcHex) != 5) {
    Serial.println("ERR BEGIN_FORMAT");
    return false;
  }
  if (width != FRAME_WIDTH || height != FRAME_HEIGHT ||
      strcmp(format, "BWR") != 0 || length != FRAME_BYTES) {
    Serial.println("ERR FRAME_CONTRACT");
    return false;
  }
  if (frameBuffer == nullptr) {
    Serial.println("ERR FRAME_BUFFER_UNAVAILABLE");
    return false;
  }

  char* crcEnd = nullptr;
  const unsigned long parsedCrc = strtoul(crcHex, &crcEnd, 16);
  if (crcEnd == crcHex || *crcEnd != '\0') {
    Serial.println("ERR CRC_FORMAT");
    return false;
  }

  expectedCrc = static_cast<uint32_t>(parsedCrc);
  receivedBytes = 0;
  runningCrc = 0xFFFFFFFFU;
  lastByteAt = millis();
  receivingFrame = true;
  Serial.printf("OK BEGIN READY bytes=%u\n", static_cast<unsigned>(FRAME_BYTES));
  return true;
}

void reply(const char* command) {
  if (strcmp(command, "HELLO WIRETERM/1") == 0) {
    Serial.println(
        "OK WIRETERM/1 state=READY render=FULL_FRAME "
        "product=WireTerm%20USB%20Device");
  } else if (strcmp(command, "STATUS") == 0) {
    Serial.printf(
        "STATUS state=READY panel=epd7in5b_V2 size=800x480 "
        "planes=BLACK,RED bytes=%u wifi=OFF bluetooth=OFF polarity=VALIDATED\n",
        static_cast<unsigned>(FRAME_BYTES));
  } else if (strcmp(command, "PINS") == 0) {
    Serial.println("PINS DIN=14 CLK=13 CS=15 DC=27 RST=26 BUSY=25 PWR=32");
  } else if (strcmp(command, "ABORT") == 0) {
    resetReceiver();
    Serial.println("OK ABORT state=READY buffered=0");
  } else if (strcmp(command, "TEST BWR") == 0) {
    Serial.println("OK TEST START expected=BLACK|RED|WHITE");
    if (displayPolarityTest()) {
      Serial.println("OK TEST COMPLETE panel_power=OFF");
    } else {
      Serial.println("ERR TEST_TIMEOUT panel_power=OFF");
    }
  } else if (strncmp(command, "BEGIN ", 6) == 0) {
    beginFrame(command);
  } else if (command[0] != '\0') {
    Serial.println("ERR UNKNOWN_COMMAND");
  }
}

void setup() {
  panelSafeOff();
  WiFi.persistent(false);
  WiFi.mode(WIFI_OFF);
  btStop();

  Serial.begin(115200);
  delay(30);
  frameBuffer = static_cast<uint8_t*>(malloc(FRAME_BYTES));
  if (frameBuffer == nullptr) {
    Serial.println("READY WIRETERM/1 state=ERROR reason=FRAME_BUFFER");
  } else {
    Serial.printf("READY WIRETERM/1 state=READY panel_power=OFF heap=%u\n",
                  static_cast<unsigned>(ESP.getFreeHeap()));
  }
}

void loop() {
  if (receivingFrame) {
    while (Serial.available() > 0 && receivedBytes < FRAME_BYTES) {
      const uint8_t value = static_cast<uint8_t>(Serial.read());
      frameBuffer[receivedBytes++] = value;
      runningCrc = updateCrc32(runningCrc, value);
      lastByteAt = millis();
    }
    if (receivedBytes == FRAME_BYTES) {
      finishFrame();
    } else if (millis() - lastByteAt >= RECEIVE_IDLE_TIMEOUT_MS) {
      Serial.printf("ERR RECEIVE_TIMEOUT buffered=%u\n",
                    static_cast<unsigned>(receivedBytes));
      resetReceiver();
    }
    return;
  }

  while (Serial.available() > 0) {
    const char value = static_cast<char>(Serial.read());
    if (value == '\n') {
      line[lineLength] = '\0';
      if (lineLength > 0 && line[lineLength - 1] == '\r') {
        line[--lineLength] = '\0';
      }
      reply(line);
      lineLength = 0;
    } else if (lineLength + 1 >= LINE_CAPACITY) {
      lineLength = 0;
      Serial.println("ERR LINE_TOO_LONG");
    } else {
      line[lineLength++] = value;
    }
  }
}
