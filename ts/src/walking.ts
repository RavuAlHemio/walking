"use strict";

import * as geojson from "geojson";
import * as leaflet from "leaflet";
import "leaflet-providers";

export module Walking {
    interface WalkingData {
        center?: [number, number],
        zoom?: number,
        track?: geojson.FeatureCollection,
        points?: geojson.FeatureCollection,
        elevation_range?: [number, number],
        heart_rate_range?: [number, number],
        speed_range?: [number, number],
        cadence_range?: [number, number],
        temperature_range?: [number, number],
    };

    interface WalkingDataFeatureProperties {
        speed: number,
        heart_rate: number,
        elevation: number,
        running_distance: number,
        cadence?: number,
        temperature?: number,
    };

    let data: WalkingData = {};
    let theMap: leaflet.Map;

    export function initializeMap(): void {
        // get the name of the map
        let queryParams = new URLSearchParams(window.location.search);
        let mapName = queryParams.get("map");
        if (mapName === null || mapName === "") {
            let mapElem = document.getElementById("the-map");
            if (mapElem !== null) {
                mapElem.textContent = "You must specify a map to load.";
            }
            return;
        }

        // construct the map URL
        let myPathPieces = window.location.pathname.split("/");
        if (myPathPieces.length > 1) {
            myPathPieces.pop();
        }
        myPathPieces.push("maps");
        myPathPieces.push(encodeURIComponent(mapName));
        let mapURL = `${window.location.origin}${myPathPieces.join("/")}.json`;

        // fetch it
        let xhr = new XMLHttpRequest();
        xhr.addEventListener("load", () => mapDownloaded(xhr));
        xhr.open("GET", mapURL, true);
        xhr.send();
    }

    function mapDownloaded(xhr: XMLHttpRequest): void {
        // store downloaded map
        data = JSON.parse(xhr.responseText);

        // set up Leaflet
        let osmLayer = obtainOsmLayer();
        let trackLayer = obtainTrackLayer();
        let elevationLayer = obtainElevationLayer();
        let heartRateLayer = obtainHeartRateLayer();
        let speedLayer = obtainSpeedLayer();
        let cadenceLayer = obtainCadenceLayer();
        let temperatureLayer = obtainTemperatureLayer();

        let layers = [osmLayer, trackLayer];
        if (heartRateLayer !== null) {
            layers.push(heartRateLayer);
        }

        theMap = leaflet.map("the-map", {
            center: data.center,
            zoom: data.zoom,
            layers: layers,
        });
        let baseMaps = {
            "OSM": osmLayer,
        };
        let overlayMaps: any = {
            "track": trackLayer,
        };
        if (heartRateLayer !== null) {
            overlayMaps["heart rate"] = heartRateLayer;
        }
        overlayMaps["elevation"] = elevationLayer;
        overlayMaps["speed"] = speedLayer;
        if (cadenceLayer !== null) {
            overlayMaps["cadence"] = cadenceLayer;
        }
        if (temperatureLayer !== null) {
            overlayMaps["temperature"] = temperatureLayer;
        }
        let layerControl = leaflet.control.layers(baseMaps, overlayMaps);
        layerControl.addTo(theMap);
    }

    function obtainOsmLayer(): leaflet.TileLayer.Provider {
        return leaflet.tileLayer.provider("OpenStreetMap.Mapnik");
    }

    function mixColor(value: number|undefined, minVal: number, maxVal: number): [number, number, number]|undefined {
        let bottomColor: [number, number, number] = [0.0, 1.0, 0.0];
        let midColor: [number, number, number] = [1.0, 1.0, 1.0];
        let topColor: [number, number, number] = [1.0, 0.0, 0.0];

        if (value === undefined) {
            return undefined;
        }

        let valFactor = (value - minVal) / (maxVal - minVal);

        let color: [number, number, number] = [0.0, 0.0, 0.0];
        if (valFactor < 0.0) {
            return bottomColor;
        } else if (valFactor < 0.5) {
            for (let i = 0; i < 3; i++) {
                color[i] = bottomColor[i] + (2*valFactor) * (midColor[i] - bottomColor[i]);
            }
        } else if (valFactor < 1.0) {
            for (let i = 0; i < 3; i++) {
                color[i] = midColor[i] + (2*(valFactor-0.5)) * (topColor[i] - midColor[i]);
            }
        } else {
            return topColor;
        }
        return color;
    }

    function hexByte(val: number): string {
        let hex = val.toString(16);
        while (hex.length < 2) {
            hex = "0" + hex;
        }
        return hex;
    }

    function hexColor(colorTuple: [number, number, number]|undefined): string {
        if (colorTuple === undefined) {
            return "#000000";
        }
        let hexTuple = colorTuple.map(v => hexByte(Math.round(v*255)));
        return "#" + hexTuple.join("");
    }

    function popup(feature: geojson.Feature, layer: leaflet.Layer) {
        let props = <WalkingDataFeatureProperties|null>feature.properties;
        if (props === null) {
            return;
        }
        let popupText
            = `<p>${props.speed.toFixed(1)} km/h</p>`
            + `<p>${props.heart_rate} BPM</p>`
            + `<p>${props.elevation.toFixed(1)} m ASL</p>`
            + `<p>${(props.running_distance/1000).toFixed(3)} km distance from beginning</p>`;
        if (props.cadence !== undefined) {
            popupText += `<p>${props.cadence} RPM cadence</p>`;
        }
        if (props.temperature !== undefined) {
            popupText += `<p>${props.temperature} \u00B0C</p>`;
        }
        layer.bindPopup(popupText);
    }

    function obtainTrackLayer(): leaflet.GeoJSON<any> {
        return leaflet.geoJSON(data.track, {});
    }

    function styleFunc(innerFunc: (props: WalkingDataFeatureProperties) => leaflet.PathOptions): leaflet.StyleFunction<geojson.GeoJsonProperties> {
        return feature => {
            if (feature === undefined) {
                return {};
            }
            let props = <WalkingDataFeatureProperties|null>feature.properties;
            if (props === null) {
                return {};
            }
            return innerFunc(props);
        };
    }

    function elevationRange(): [number, number] {
        let dataRange = data.elevation_range;
        if (dataRange !== undefined) {
            return dataRange;
        }
        return [300, 400];
    }

    function speedRange(): [number, number] {
        let dataRange = data.speed_range;
        if (dataRange !== undefined) {
            return dataRange;
        }
        return [0, 10];
    }

    function haveLayer(valueFunc: (props: { [name: string]: any }) => any): boolean {
        if (data.points === undefined) {
            return false;
        }

        let haveValue = false;
        for (let feature of data.points.features) {
            if (feature.properties === null) {
                continue;
            }

            if (valueFunc(feature.properties) !== undefined) {
                haveValue = true;
                break;
            }
        }
        return haveValue;
    }

    function obtainElevationLayer(): leaflet.GeoJSON<any> {
        return leaflet.geoJSON(data.points, {
            style: styleFunc(props => ({
                color: hexColor(mixColor(props.elevation, elevationRange()[0], elevationRange()[1])),
                opacity: 1,
                weight: 4,
            })),
            onEachFeature: popup,
        });
    }

    function obtainHeartRateLayer(): leaflet.GeoJSON<any>|null {
        if (!haveLayer(props => props.heart_rate)) {
            return null;
        }
        return leaflet.geoJSON(data.points, {
            style: styleFunc(props => ({
                color: hexColor(mixColor(props.heart_rate, 80, 160)),
                opacity: 1,
                weight: 8,
            })),
            onEachFeature: popup,
        });
    }

    function obtainSpeedLayer(): leaflet.GeoJSON<any> {
        return leaflet.geoJSON(data.points, {
            style: styleFunc(props => ({
                color: hexColor(mixColor(props.speed, speedRange()[0], speedRange()[1])),
                opacity: 1,
                weight: 4,
            })),
            onEachFeature: popup,
        });
    }

    function obtainCadenceLayer(): leaflet.GeoJSON<any>|null {
        if (!haveLayer(props => props.cadence)) {
            return null;
        }

        return leaflet.geoJSON(data.points, {
            style: styleFunc(props => ({
                color: hexColor(mixColor(props.cadence, 0, 120)),
                opacity: 1,
                weight: 4,
            })),
            onEachFeature: popup,
        });
    }

    function obtainTemperatureLayer(): leaflet.GeoJSON<any>|null {
        if (!haveLayer(props => props.temperature)) {
            return null;
        }

        return leaflet.geoJSON(data.points, {
            style: styleFunc(props => ({
                color: hexColor(mixColor(props.temperature, -10, 45)),
                opacity: 1,
                weight: 4,
            })),
            onEachFeature: popup,
        });
    }
}

document.addEventListener("DOMContentLoaded", () => Walking.initializeMap());
